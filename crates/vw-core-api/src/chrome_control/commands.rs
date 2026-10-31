//! Browser availability and connection setup for Settings and the Debug API.
//! Called by core dispatch; permissions come from chat access modes.

use super::setup;
use crate::install_offers::{OfferKey, ToolsInstallOffer};
use crate::{CommandResult, Core, CoreError, ErrorCode, Origin};

/// What the window asks of Chrome Control.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChromeControlRequest {
    /// Availability, connection and chat work states.
    Read {},
    /// Claim one selected-text item for the composer.
    TakeSelection {
        /// The pending selection identity.
        selection_id: String,
    },
    /// List connected tabs without opening a chat window.
    Tabs {},
    /// Read the exact tab selected by the person.
    TabContext {
        /// Connected profile identity.
        profile_id: String,
        /// Exact tab identity.
        tab_id: u32,
    },
    /// Turn Chrome tools on or off.
    SetEnabled {
        /// Whether Chrome tools are available.
        enabled: bool,
    },
    /// Settings > Developer: let the agent use local-network pages in Chrome (off by default).
    SetLocalNetwork {
        /// Whether local-network pages are allowed for the agent in Chrome.
        enabled: bool,
    },
    /// What Set up installs, as an offer the click confirms.
    OfferSetup {},
    /// Set up, confirming the offer the window showed.
    Setup {
        /// The offer's id.
        offer_id: String,
    },
    /// Open the extension's store page in Chrome.
    OpenStore {},
    /// Remove what Set up made.
    Remove {},
}

/// Settings › Browser › Your Chrome.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChromeControlView {
    /// Whether Chrome tools are available.
    pub enabled: bool,
    /// Settings > Developer: local-network pages are allowed for the agent in Chrome.
    pub local_network: bool,
    /// The running helper acknowledged `local_network`'s current value (false while a change is still being sent).
    pub local_network_applied: bool,
    /// Recent chat work, bounded and independent of permissions.
    pub work: Vec<super::ChromeWorkView>,
    /// Pending selected-text deliveries, claimed once by a composer.
    pub pending_selections: Vec<String>,
    /// A bounded delivery failure state.
    pub selection_error: Option<String>,
    /// Where the setup stands.
    pub setup: setup::ChromeSetupView,
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}

const OFFER_KIND: &str = "chromeControl";

/// The store listing is in Google's review: the last manifest the Vibe installer accepted says `"review"`.
/// No installer, no manifest, `"available"` or an unknown value: the normal store link.
pub(crate) fn store_review(shared: &crate::core::Shared) -> bool {
    shared
        .config
        .vibe_install
        .as_ref()
        .is_some_and(|installer| {
            installer.extension_store() == Some(vw_vibe_install::ExtensionStore::Review)
        })
}

pub(crate) fn view(shared: &crate::core::Shared) -> ChromeControlView {
    let chrome = &shared.chrome_control;
    let mut setup = setup::view(&chrome.config, chrome.dir(), &shared.instructions.own_home);
    setup.store_review = setup.state != setup::SetupState::Absent && store_review(shared);
    let pairing = chrome
        .owner
        .pairing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if setup.package && !pairing.0.is_empty() {
        setup.pairing.clone_from(&pairing.0);
    }
    if setup.package {
        setup.fingerprint.clone_from(&pairing.1);
    }
    let selections = chrome
        .selections
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ChromeControlView {
        pending_selections: selections.ids(),
        selection_error: selections.error.clone(),
        enabled: chrome.enabled(),
        local_network: chrome.local_network(),
        local_network_applied: chrome.local_network_applied(),
        work: chrome
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .view(),
        setup,
    }
}

impl Core {
    /// Settings > Developer > Local network pages for the agent in Chrome.
    async fn set_local_network(&self, enabled: bool) -> Result<CommandResult, CoreError> {
        self.shared
            .chrome_control
            .set_local_network(enabled)
            .map_err(|reason| invalid(&reason))?;
        // Chrome's live tabs take the switch at once (the helper holds new actions until Chrome confirms).
        super::send_local_network(&self.shared).await;
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }

    pub(super) fn chrome_view(&self) -> ChromeControlView {
        view(&self.shared)
    }

    fn chrome_result(&self, offer: Option<ToolsInstallOffer>) -> CommandResult {
        CommandResult::ChromeControl {
            view: self.chrome_view(),
            offer,
        }
    }

    fn setup_offer(&self) -> Result<ToolsInstallOffer, CoreError> {
        let config = &self.shared.chrome_control.config;
        setup::packaged_helper(config).map_err(|reason| invalid(&reason))?;
        // In Google's review the store page is not opened (the setup says so instead), so the offer does not say it.
        let store = if store_review(&self.shared) {
            String::new()
        } else {
            format!(
                " Opens the Chrome Web Store page so you can add the {} extension.",
                setup::STORE_LISTING_NAME
            )
        };
        self.shared.install_offers.make(
            OfferKey::new(OFFER_KIND, setup::PLUGIN, ""),
            ToolsInstallOffer {
                offer_id: String::new(),
                kind: OFFER_KIND.to_owned(),
                name: "Chrome Control".to_owned(),
                publisher: Some("Vibe Work".to_owned()),
                source: "Shipped with Vibe Work".to_owned(),
                licence: None,
                // The store and Chrome show the extension under its listing name, not "Chrome Control".
                runs: format!(
                    "Installs the Chrome Control helper in Vibe Work's own folder, for your user only (no \
                     administrator), tells Google Chrome where it is, and connects Chrome Control to Vibe Work.{store}"
                ),
                version: None,
                expires_at: 0,
            },
            self.shared.asks.now_ms(),
        )
    }

    /// `chromeControl` `setup`: takes the one-time offer, stores the owner identity and installs the helper.
    async fn chrome_setup(&self, offer_id: String) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        if chrome
            .dir()
            .is_some_and(|dir| dir.join("setup.json").is_file())
        {
            return Err(invalid("Chrome Control is already set up."));
        }
        self.shared.install_offers.take(
            &offer_id,
            &OfferKey::new(OFFER_KIND, setup::PLUGIN, ""),
            self.shared.asks.now_ms(),
        )?;
        let state = chrome.dir().ok_or_else(|| {
            invalid("Could not set up Chrome Control: Vibe Work has no app folder.")
        })?;
        let keys = self
            .shared
            .config
            .keys
            .as_ref()
            .ok_or_else(|| invalid("Chrome Control needs the OS credential store."))?;
        keys.create_chrome_owner().await.map_err(|_| {
            invalid("Could not store Chrome Control's owner identity. Nothing was changed.")
        })?;
        if let Err(reason) =
            setup::install(&chrome.config, state, &self.shared.instructions.own_home)
        {
            let _ = keys.delete_chrome_owner().await;
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!("Could not set up Chrome Control: {reason}. Nothing was changed."),
            ));
        }
        super::start_owner(&self.shared).await;
        tracing::info!("Chrome Control set up");
        if store_review(&self.shared) {
            tracing::info!("the store listing is in review: the store page is not opened");
        } else if let Err(problem) = setup::open_store(&chrome.config) {
            tracing::warn!(%problem, "the Chrome Web Store page could not be opened");
        }
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }

    async fn chrome_remove(&self) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid("Vibe Work has no app folder."))?;
        super::stop_owner(&self.shared).await;
        if let Some(keys) = &self.shared.config.keys {
            keys.delete_chrome_owner()
                .await
                .map_err(|_| invalid("Could not delete Chrome Control's owner identity."))?;
        }
        let removed = setup::remove(state);
        chrome.reset();
        removed.map_err(|reason| {
            CoreError::new(
                ErrorCode::Internal,
                format!("Could not remove everything of Chrome Control: {reason}"),
            )
        })?;
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }

    /// `chromeControl`.
    ///
    /// # Errors
    /// `invalidParams`/`staleOffer` for a used or expired setup offer; `internal` when
    /// saving availability, setup or removal fails.
    pub(crate) async fn chrome_control(
        &self,
        _origin: Origin,
        request: ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        if !chrome.config.available() && !matches!(request, ChromeControlRequest::Read {}) {
            return Err(invalid(setup::ABSENT));
        }
        match request {
            ChromeControlRequest::Read {} => Ok(self.chrome_result(None)),
            ChromeControlRequest::TakeSelection { selection_id } => {
                let mut selection = chrome
                    .selections
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take(&selection_id);
                if let Some(selection) = &mut selection {
                    selection.keep_live_chat(|id| self.shared.store.view(id).is_some());
                }
                super::owner::publish(&self.shared);
                Ok(CommandResult::ChromeSelection { selection })
            }
            ChromeControlRequest::Tabs {} => super::tabs::read(&self.shared, chrome).await,
            ChromeControlRequest::TabContext { profile_id, tab_id } => {
                super::tabs::pick(&self.shared, chrome, &profile_id, tab_id).await
            }
            ChromeControlRequest::SetEnabled { enabled } => {
                chrome
                    .set_enabled(enabled)
                    .map_err(|reason| invalid(&reason))?;
                if enabled {
                    super::start_owner(&self.shared).await;
                } else {
                    super::stop_owner(&self.shared).await;
                    chrome.reset();
                }
                self.shared.emit(crate::CoreEvent::ChromeControl {
                    view: self.chrome_view(),
                });
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::SetLocalNetwork { enabled } => {
                self.set_local_network(enabled).await
            }
            ChromeControlRequest::OfferSetup {} => {
                let offer = self.setup_offer()?;
                Ok(self.chrome_result(Some(offer)))
            }
            ChromeControlRequest::Setup { offer_id } => self.chrome_setup(offer_id).await,
            ChromeControlRequest::OpenStore {} => {
                setup::open_store(&chrome.config)
                    .map_err(|reason| CoreError::new(ErrorCode::Internal, reason))?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::Remove {} => self.chrome_remove().await,
        }
    }
}
