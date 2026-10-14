//! The Tools marketplace in the core: the `marketplace`
//! command. Reads the curated catalog with what is installed and what the viewed session runs; installs (in the
//! background, one at a time) and removes. Vibe offers no per-skill or per-session switch for plugin skills (they are
//! `locked`), so install/remove is the switch.
//!
//! Depends on: vw-market (catalog, installer, store), `market_view` (wire types), `market_project` (Vibe's
//! replies -> views), `channels`, the gate, `dispatch::forward`, `worktrees::ask_native` (the native dialog).
//! Called by: `dispatch` (`Command::Marketplace`).
//! Invariants:
//! - Installs land in Vibe Work's own Vibe home (`instructions.own_home`), never `~/.vibe`.
//! - An install asks the native dialog from the window; the Debug API acts as the user
//!   (answered in `confirmer`). The dialog names the entry, its publisher, licence and what runs.
//! - At most one install holds the slot, taken in one locked step before the dialog (`MarketState::reserve`); its
//!   task holds the core while it runs and is aborted on cancel and at shutdown (its staging folder is removed by
//!   the installer's guard).
//! - After an install or removal every session with an open channel gets `plugin/reload` (Vibe re-pins plugins);
//!   a failing reload is logged, never fatal. Reads never reopen a session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_market::{Catalog, Fetcher, HttpFetcher, Installer, Kind, MarketError, Rules, Store};

use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::market_view::{InstallProgress, MarketRequest, MarketResult, MarketView};

/// The marketplace settings of a core.
#[derive(Clone, Default)]
pub struct MarketConfig {
    /// A catalog file to use instead of the built-in one (tests and development only; the app leaves it `None`).
    pub index: Option<PathBuf>,
    /// Accept loopback `http://` addresses in that catalog (tests only).
    pub allow_loopback_http: bool,
    /// A downloader to use instead of the HTTPS one (tests).
    pub fetcher: Option<Arc<dyn Fetcher>>,
}

/// The catalog, the downloader and the running install.
pub(crate) struct MarketState {
    catalog: Result<Arc<Catalog>, String>,
    fetcher: Result<Arc<dyn Fetcher>, String>,
    /// The install that holds the one slot: reserved (before its dialog is answered) or running (its task).
    running: Mutex<Option<Slot>>,
    progress: Mutex<Option<InstallProgress>>,
}

impl MarketState {
    /// Loads the catalog named by `config` (the built-in one by default).
    pub(crate) fn new(config: &MarketConfig) -> Self {
        let rules = Rules {
            allow_loopback_http: config.allow_loopback_http,
            require_all_targets: config.index.is_none(),
        };
        let catalog = match &config.index {
            None => Catalog::built_in(),
            Some(path) => std::fs::read_to_string(path)
                .map_err(|e| MarketError::Catalog(format!("not readable: {e}")))
                .and_then(|text| Catalog::parse(&text, rules)),
        }
        .map(Arc::new)
        .map_err(|e| e.to_string());
        if let Err(error) = &catalog {
            warn!(%error, "marketplace: the catalog was refused");
        }
        let fetcher = match &config.fetcher {
            Some(fetcher) => Ok(fetcher.clone()),
            None => HttpFetcher::new(config.allow_loopback_http)
                .map(|f| Arc::new(f) as Arc<dyn Fetcher>)
                .map_err(|e| e.to_string()),
        };
        Self {
            catalog,
            fetcher,
            running: Mutex::new(None),
            progress: Mutex::new(None),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Slot>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The install holding the slot (reserved or running), if any.
    fn running(&self) -> Option<String> {
        self.slot()
            .as_ref()
            .filter(|slot| slot.task.as_ref().is_none_or(|task| !task.is_finished()))
            .map(|slot| slot.id.clone())
    }

    /// Takes the slot for `id` in one locked step (two concurrent installs must not both pass a
    /// separate check and then both ran on one staging folder). Returns the ticket, or the id holding it.
    fn reserve(&self, id: &str) -> Result<u64, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICKETS: AtomicU64 = AtomicU64::new(1);
        let mut slot = self.slot();
        if let Some(held) = slot.as_ref()
            && held.task.as_ref().is_none_or(|task| !task.is_finished())
        {
            return Err(held.id.clone());
        }
        let ticket = TICKETS.fetch_add(1, Ordering::Relaxed);
        *slot = Some(Slot {
            id: id.to_owned(),
            ticket,
            task: None,
        });
        Ok(ticket)
    }

    /// Gives back a reservation that did not start (declined dialog, error).
    fn release(&self, ticket: u64) {
        let mut slot = self.slot();
        if slot
            .as_ref()
            .is_some_and(|s| s.ticket == ticket && s.task.is_none())
        {
            *slot = None;
        }
    }

    /// Attaches the started task to its reservation; false when the reservation is gone (cancelled meanwhile).
    fn attach(&self, ticket: u64, task: JoinHandle<()>) -> bool {
        let mut slot = self.slot();
        match slot.as_mut() {
            Some(held) if held.ticket == ticket && held.task.is_none() => {
                held.task = Some(task);
                true
            }
            _ => {
                task.abort();
                false
            }
        }
    }

    /// Ends the slot of the task with `ticket` (the task's own last step).
    fn finish(&self, ticket: u64) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|s| s.ticket == ticket) {
            *slot = None;
        }
    }

    fn set_progress(&self, progress: Option<InstallProgress>) {
        *self.progress.lock().unwrap_or_else(PoisonError::into_inner) = progress;
    }

    fn progress(&self) -> Option<InstallProgress> {
        self.progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The slot's holder.
struct Slot {
    id: String,
    ticket: u64,
    task: Option<JoinHandle<()>>,
}

impl Drop for MarketState {
    fn drop(&mut self) {
        if let Some(task) = self.slot().take().and_then(|s| s.task) {
            task.abort();
        }
    }
}

/// Aborts a running install (core shutdown).
pub(crate) fn abort_install(shared: &Shared) {
    if let Some(task) = shared.market.slot().take().and_then(|s| s.task) {
        task.abort();
    }
}

fn from_market(error: &MarketError) -> CoreError {
    let code = match error {
        MarketError::InvalidId(_)
        | MarketError::UnknownEntry(_)
        | MarketError::NotInstallable(_)
        | MarketError::Foreign(_)
        | MarketError::NotInstalled(_) => ErrorCode::InvalidParams,
        _ => ErrorCode::Internal,
    };
    let mut mapped = CoreError::new(code, error.to_string());
    mapped.server_code = Some(error.code().to_owned());
    mapped
}

impl Core {
    /// Runs one `marketplace` request for `origin`.
    ///
    /// # Errors
    /// `invalidParams` (unknown entry, a server entry, a foreign folder, not installed, a bad skill name),
    /// `denied` with `serverCode: "installRunning"` while an install runs, `confirmationDeclined`, `internal` when
    /// the catalog is unavailable.
    pub(crate) async fn marketplace(
        &self,
        origin: Origin,
        request: MarketRequest,
    ) -> Result<CommandResult, CoreError> {
        let result = match request {
            MarketRequest::Read { session_id } => MarketResult::View {
                view: Box::new(self.market_view(session_id).await),
                notice: None,
            },
            MarketRequest::Install { id } => self.start_install(origin, id).await?,
            MarketRequest::CancelInstall {} => {
                let taken = self.shared.market.slot().take();
                if let Some(slot) = taken {
                    // The task's staging guard deletes its own staging folder when the aborted future drops.
                    if let Some(task) = slot.task {
                        task.abort();
                        let _ = task.await;
                    }
                    self.report(&slot.id, "cancelled", String::new(), None);
                }
                MarketResult::View {
                    view: Box::new(self.market_view(None).await),
                    notice: None,
                }
            }
            MarketRequest::Remove { id } => {
                if self.shared.market.running().as_deref() == Some(id.as_str()) {
                    return Err(running_error(&id));
                }
                let store = self.store();
                let servers = store.mcp_server_names(&id).map_err(|e| from_market(&e))?;
                store.remove(&id, false).map_err(|e| from_market(&e))?;
                for name in servers {
                    self.shared.connectors.mark_removed(&name, true);
                }
                info!(id, "marketplace: removed");
                self.reload_sessions().await;
                let notice = store.remove_data(&id).err().map(|error| {
                    warn!(id, %error, "marketplace: removed; data folder remains");
                    format!("Removed; could not delete data folder \"{id}\".")
                });
                MarketResult::View {
                    view: Box::new(self.market_view(None).await),
                    notice,
                }
            }
        };
        Ok(CommandResult::Marketplace { result })
    }

    fn store(&self) -> Store {
        Store::new(&self.shared.instructions.own_home)
    }

    fn catalog(&self) -> Result<Arc<Catalog>, CoreError> {
        self.shared
            .market
            .catalog
            .clone()
            .map_err(|e| CoreError::new(ErrorCode::Internal, e))
    }

    /// `install`: checks, the native dialog, then the owned task.
    async fn start_install(&self, origin: Origin, id: String) -> Result<MarketResult, CoreError> {
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
        let checked = self.check_and_confirm(origin, &id, &entry).await;
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
            let outcome = installer.install(&task_id, &on).await;
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

    /// The checks after the slot is reserved: not a foreign folder, a downloader, then the native dialog.
    async fn check_and_confirm(
        &self,
        origin: Origin,
        id: &str,
        entry: &vw_market::Entry,
    ) -> Result<(Store, Arc<dyn Fetcher>), CoreError> {
        let store = self.store();
        store.receipt(id).map_err(|e| from_market(&e))?;
        let fetcher = self
            .shared
            .market
            .fetcher
            .clone()
            .map_err(|e| CoreError::new(ErrorCode::Internal, e))?;
        let summary = crate::market_project::consent_summary(entry);
        self.ask_native(
            crate::command::requester(origin),
            "vw/marketplace/install",
            json!({"id": id}),
            summary,
        )
        .await?;
        Ok((store, fetcher))
    }

    /// One progress step from the installer: downloads are reported at most every 250 ms.
    fn on_step(&self, id: &str, step: &vw_market::Step, last: &Mutex<Instant>) {
        if step.phase == "download" && step.done.unwrap_or(0) > 0 {
            let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
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
    fn report(&self, id: &str, phase: &str, what: String, message: Option<String>) {
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

    /// `plugin/reload` for every session with an open channel.
    async fn reload_sessions(&self) {
        for open in self.shared.channels.open_channels() {
            let params = json!({"sessionId": open.session_id});
            if let Err(error) = self.shared.gate.call("plugin/reload", params, None).await {
                warn!(%error, "marketplace: a session did not reload its plugins");
            }
        }
    }

    /// The view: the catalog, the receipts, and the state of `session_id` when it has an open channel.
    async fn market_view(&self, session_id: Option<String>) -> MarketView {
        let catalog = match &self.shared.market.catalog {
            Ok(catalog) => catalog.clone(),
            Err(error) => return crate::market_project::unavailable(error),
        };
        let open = session_id.filter(|id| self.shared.channels.has_channel(id));
        let (plugins, skills) = match &open {
            Some(id) => (
                self.session_read("plugin_catalog/read", id).await,
                self.session_read("skills/installed", id).await,
            ),
            None => (Value::Null, Value::Null),
        };
        crate::market_project::view(crate::market_project::Inputs {
            catalog: &catalog,
            store: &self.store(),
            plugins: &plugins,
            skills: &skills,
            session_id: open,
            installing: self.shared.market.progress(),
        })
    }

    async fn session_read(&self, method: &str, session_id: &str) -> Value {
        match self
            .shared
            .gate
            .call(method, json!({"sessionId": session_id}), None)
            .await
        {
            Ok(reply) => reply.result,
            Err(error) => {
                warn!(method, %error, "marketplace: the session's state was not read");
                Value::Null
            }
        }
    }
}

fn running_error(id: &str) -> CoreError {
    let mut error = CoreError::new(
        ErrorCode::Denied,
        format!("the install of \"{id}\" is still running: wait for it or stop it first"),
    );
    error.server_code = Some("installRunning".to_owned());
    error
}

/// True when the entry can be installed on this computer (packs always; plugins when a runtime exists for it).
pub(crate) fn installable(entry: &vw_market::Entry) -> bool {
    match (&entry.kind, &entry.plugin) {
        (Kind::Pack, _) => true,
        (Kind::Plugin, Some(plugin)) => {
            vw_market::current_target().is_some_and(|t| plugin.runtime.targets.contains_key(t))
        }
        _ => false,
    }
}
