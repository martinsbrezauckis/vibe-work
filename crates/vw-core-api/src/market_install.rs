//! One marketplace install: the slot, checks and native dialog (or the window's click on a shown offer),
//! the installer task with its progress steps, and the phase and outcome events the composer shows.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vw_platform::sync::LockRecover;

use serde_json::json;
use tracing::{info, warn};
use vw_market::{Fetcher, Installer, Kind, MarketError, Store};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::market_view::{InstallProgress, MarketResult};

use super::{from_market, installable};

pub(super) fn running_error(id: &str) -> CoreError {
    let mut error = CoreError::new(
        ErrorCode::Denied,
        format!("the install of \"{id}\" is still running: wait for it or stop it first"),
    );
    error.server_code = Some("installRunning".to_owned());
    error
}

impl Core {
    /// `install`: checks, the native dialog (or the window's click on a shown offer), then the owned task.
    pub(super) async fn start_install(
        &self,
        origin: Origin,
        id: String,
        offer_id: Option<String>,
    ) -> Result<MarketResult, CoreError> {
        let offered = self.take_market_offer(origin, &id, offer_id.as_deref())?;
        let catalog = self.catalog()?;
        let entry = catalog
            .entry(&id)
            .ok_or_else(|| from_market(&MarketError::UnknownEntry(id.clone())))?
            .clone();
        if entry.kind == Kind::Server {
            return Err(from_market(&MarketError::NotInstallable(
                "a remote server is added from the Connectors page, not installed".to_owned(),
            )));
        }
        if !installable(&entry) {
            return Err(from_market(&MarketError::NotInstallable(
                "this computer's platform has no runtime for it in the catalog".to_owned(),
            )));
        }
        let ticket = self
            .shared
            .market
            .reserve(&id)
            .map_err(|running| running_error(&running))?;
        let checked = self.check_and_confirm(origin, &id, &entry, offered).await;
        let (store, fetcher) = match checked {
            Ok(ready) => ready,
            Err(error) => {
                self.shared.market.release(ticket);
                return Err(error);
            }
        };
        let (go, wait) = tokio::sync::oneshot::channel::<()>();
        let weak = Arc::downgrade(&self.shared);
        let task_id = id.clone();
        self.report(&id, "download", String::new(), None);
        let task = tokio::spawn(async move {
            // Starts only once it holds the slot (attached below), so a fast failure cannot end the slot first.
            if wait.await.is_err() {
                return;
            }
            let Some(shared) = weak.upgrade() else { return };
            let core = Core { shared };
            let last = Mutex::new(Instant::now());
            let on = |step: vw_market::Step| core.on_step(&task_id, &step, &last);
            let installer = Installer {
                catalog: &catalog,
                store: &store,
                fetcher: fetcher.as_ref(),
                target: vw_market::current_target(),
            };
            let commit = |id: &str, root: &std::path::Path| {
                core.shared
                    .plugin_approvals
                    .commit_install(&store, id, root)
            };
            let outcome = installer.install_with_commit(&task_id, &on, &commit).await;
            match outcome {
                Ok(receipt) => {
                    // These are the same names the installer wrote to mcp.json; clear only on success.
                    for name in entry
                        .pack
                        .iter()
                        .flat_map(|pack| pack.mcp_servers.keys())
                        .chain(
                            entry
                                .plugin
                                .iter()
                                .flat_map(|plugin| plugin.mcp_servers.keys()),
                        )
                    {
                        core.shared.connectors.mark_removed(name, false);
                    }
                    core.shared.market.finish(ticket);
                    info!(
                        id = task_id,
                        version = receipt.version,
                        "marketplace: installed"
                    );
                    core.reload_sessions().await;
                    core.report(&task_id, "installed", String::new(), None);
                }
                Err(error) => {
                    core.shared.market.finish(ticket);
                    warn!(
                        id = task_id,
                        code = error.code(),
                        "marketplace: install failed"
                    );
                    core.report(&task_id, "failed", String::new(), Some(error.to_string()));
                }
            }
        });
        if !self.shared.market.attach(ticket, task) {
            // Cancelled while the dialog was open: nothing was started.
            return Err(CoreError::new(
                ErrorCode::Cancelled,
                "the install was stopped before it started",
            ));
        }
        let _ = go.send(());
        Ok(MarketResult::InstallStarted { id })
    }

    /// The checks after the slot is reserved: not a foreign folder, a downloader, then the native dialog unless
    /// the window's click confirmed a shown offer (`offered`).
    async fn check_and_confirm(
        &self,
        origin: Origin,
        id: &str,
        entry: &vw_market::Entry,
        offered: bool,
    ) -> Result<(Store, Arc<dyn Fetcher>), CoreError> {
        let store = self.store();
        store.receipt(id).map_err(|e| from_market(&e))?;
        let off = store.plugins_dir().with_file_name("plugins-off").join(id);
        if off.exists() && crate::market_local::receipt(&off).is_none() {
            return Err(from_market(&MarketError::Foreign(id.to_owned())));
        }
        if crate::market_local::receipt(&off).is_some() {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "This plugin is turned off. Turn it on before updating it.",
            ));
        }

        let fetcher = self
            .shared
            .market
            .fetcher
            .clone()
            .map_err(|e| CoreError::new(ErrorCode::Internal, e))?;
        if offered {
            info!(
                id,
                "marketplace: the window's click confirmed the shown offer"
            );
        } else {
            let summary = crate::market_project::consent_summary(entry);
            self.ask_native(
                crate::command::requester(origin),
                "vw/marketplace/install",
                json!({"id": id}),
                summary,
            )
            .await?;
        }
        Ok((store, fetcher))
    }

    /// Takes the window's install offer for `id`, first (single use, also when a later check fails). True when the
    /// click may skip the native dialog: only the window's; the Debug API keeps its confirmer path.
    fn take_market_offer(
        &self,
        origin: Origin,
        id: &str,
        offer_id: Option<&str>,
    ) -> Result<bool, CoreError> {
        let Some(offer) = offer_id else {
            return Ok(false);
        };
        self.shared.install_offers.take(
            offer,
            &crate::install_offers::OfferKey::new("vw/marketplace/install", id, ""),
            self.shared.asks.now_ms().into(),
        )?;
        Ok(origin == Origin::Ui)
    }

    /// `offerInstall`: the catalog's own name, publisher and licence, and what the install puts on this computer
    /// (the same sentence the native dialog shows), kept as a single-use offer for `install`.
    pub(super) fn market_offer(
        &self,
        id: &str,
    ) -> Result<crate::install_offers::ToolsInstallOffer, CoreError> {
        let catalog = self.catalog()?;
        let entry = catalog
            .entry(id)
            .ok_or_else(|| from_market(&MarketError::UnknownEntry(id.to_owned())))?;
        if entry.kind == Kind::Server || !installable(entry) {
            return Err(from_market(&MarketError::NotInstallable(
                "there is nothing to install for it on this computer".to_owned(),
            )));
        }
        let offer = crate::install_offers::ToolsInstallOffer {
            offer_id: String::new(),
            kind: "marketplace".to_owned(),
            name: entry.name.clone(),
            publisher: Some(entry.publisher.clone()),
            source: "Vibe Work's reviewed Tools catalog".to_owned(),
            licence: Some(entry.licence.clone()),
            runs: crate::market_project::consent_summary(entry),
            version: None,
            expires_at: 0,
        };
        self.shared.install_offers.make(
            crate::install_offers::OfferKey::new("vw/marketplace/install", id, ""),
            offer,
            self.shared.asks.now_ms().into(),
        )
    }

    /// One progress step from the installer: downloads are reported at most every 250 ms.
    fn on_step(&self, id: &str, step: &vw_market::Step, last: &Mutex<Instant>) {
        if step.phase == "download" && step.done.unwrap_or(0) > 0 {
            let mut last = last.lock_or_recover();
            if last.elapsed() < Duration::from_millis(250) {
                return;
            }
            *last = Instant::now();
        }
        let progress = InstallProgress {
            id: id.to_owned(),
            phase: step.phase.to_owned(),
            what: step.what.clone(),
            done: step.done,
            total: step.total,
            message: None,
        };
        self.shared.market.set_progress(Some(progress.clone()));
        self.shared.emit(CoreEvent::MarketInstall { progress });
    }

    /// Records and sends a phase or an outcome (`installed`, `failed`, `cancelled`).
    pub(super) fn report(&self, id: &str, phase: &str, what: String, message: Option<String>) {
        let progress = InstallProgress {
            id: id.to_owned(),
            phase: phase.to_owned(),
            what,
            done: None,
            total: None,
            message,
        };
        let finished = matches!(phase, "installed" | "failed" | "cancelled");
        self.shared.market.set_progress(if finished {
            None
        } else {
            Some(progress.clone())
        });
        self.shared.emit(CoreEvent::MarketInstall { progress });
    }
}
