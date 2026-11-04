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
    /// Continue installation after setup, using Chrome's native prompt where supported.
    Connect {},
    /// Offer the shared browser setup with the store opened in Edge.
    OfferEdgeSetup {},
    /// Confirm the one-time Edge setup offer.
    SetupEdge {
        /// The offer shown to the person.
        offer_id: String,
    },
    /// Open the unchanged extension listing in Edge.
    OpenEdgeStore {},
    /// Offer the optional Firefox connection.
    OfferFirefoxSetup {},
    /// Confirm the Firefox setup offer.
    SetupFirefox {
        /// The offer shown to the person.
        offer_id: String,
    },
    /// Open the approved Firefox add-on listing.
    OpenFirefoxStore {},
    /// Open or reuse the app-owned Firefox window.
    OpenFirefox {},
    /// The person closes the app-owned Firefox window.
    CloseFirefox {},
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
    /// Edge's own setup and pairing state, absent while the optional module is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub edge: Option<Box<setup::ChromeSetupView>>,
    /// Firefox setup, omitted while the optional module is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub firefox: Option<Box<setup::ChromeSetupView>>,
}

pub(super) fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}

const OFFER_KIND: &str = "chromeControl";

/// The store listing is in Google's review: the last manifest the Vibe installer accepted says `"review"`.
/// No installer, no manifest, `"available"` or an unknown value: the normal store link.
pub(crate) fn store_review(shared: &crate::core::Shared) -> bool {
    extension_store(shared) == Some(vw_vibe_install::ExtensionStore::Review)
}

/// Last accepted signed release state; absence is unknown, not evidence that the listing is available.
pub(crate) fn extension_store(
    shared: &crate::core::Shared,
) -> Option<vw_vibe_install::ExtensionStore> {
    shared
        .config
        .vibe_install
        .as_ref()
        .and_then(vw_vibe_install::VibeInstaller::extension_store)
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
    if setup.package && setup.state != setup::SetupState::RepairRequired && !pairing.0.is_empty() {
        setup.pairing.clone_from(&pairing.0);
    }
    if setup.state == setup::SetupState::WaitingForExtension && setup.pairing == "Paired" {
        setup.state = setup::SetupState::ExtensionAdded;
    }
    if setup.package {
        setup.fingerprint.clone_from(&pairing.1);
    }
    let pairing_text = pairing.0.clone();
    drop(pairing);
    setup.addon = Some(addon_view(shared, &setup, &pairing_text));
    let left = extension_left(chrome);
    let external = chrome
        .removed_external
        .load(std::sync::atomic::Ordering::SeqCst);
    setup.extension_left = (left && !external).then_some(true);
    setup.extension_restart = (left && external).then_some(true);
    let selections = chrome
        .selections
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let edge = cfg!(feature = "your-edge").then(|| {
        let mut edge = setup::view_for(
            &chrome.config,
            chrome.dir(),
            &shared.instructions.own_home,
            "Microsoft Edge",
        );
        edge.store_review = setup.store_review;
        let pairing = chrome
            .owner
            .edge_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Like Chrome, a broken shared helper keeps its reinstall diagnosis ahead of pairing state.
        if edge.package && edge.state != setup::SetupState::RepairRequired {
            edge.pairing = if pairing.0.is_empty() {
                vw_vocabulary::texts::EDGE_NOT_CONNECTED.into()
            } else {
                pairing.0.clone()
            };
            edge.fingerprint.clone_from(&pairing.1);
        }
        if edge.state == setup::SetupState::WaitingForExtension && edge.pairing == "Paired" {
            edge.state = setup::SetupState::ExtensionAdded;
        }
        if let Some(reason) = super::edge_policy::refusal(&chrome.config) {
            edge.pairing = reason.into();
        }
        drop(pairing);
        edge.addon = Some(super::agent_browser::edge_addon(shared, &edge));
        Box::new(edge)
    });
    ChromeControlView {
        firefox: super::firefox_commands::setup_view(shared),
        edge,
        pending_selections: selections.ids(),
        selection_error: selections.error.clone(),
        enabled: chrome.enabled(),
        local_network: chrome.local_network(),
        local_network_applied: chrome.local_network_applied(),
        work: super::work::view(chrome),
        setup,
    }
}

/// Your Chrome's state now (the card's and the agent's).
pub(crate) fn addon(shared: &crate::core::Shared) -> super::AddonView {
    view(shared)
        .setup
        .addon
        .unwrap_or_else(|| super::addon::view(super::AddonState::NotConnected, ""))
}

/// Your Chrome's one state (`addon`), from the live connection first and Chrome's files last.
fn addon_view(
    shared: &crate::core::Shared,
    setup: &setup::ChromeSetupView,
    pairing: &str,
) -> super::AddonView {
    if setup.state == setup::SetupState::RepairRequired {
        let mut view = super::addon::view(super::AddonState::HelperSetupRequired, "");
        view.extension_version =
            super::installation::version(shared, super::agent_browser::Browser::Chrome);
        return view;
    }
    let chrome = &shared.chrome_control;
    let (waiting, connecting) = {
        let enroll = chrome
            .owner
            .enroll
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            enroll.waiting_reported(),
            enroll.connecting(tokio::time::Instant::now()),
        )
    };
    let user_data = setup::user_data(chrome.dir(), "Google Chrome");
    let profiles: Vec<std::path::PathBuf> = user_data
        .iter()
        .flat_map(|dir| super::probe::profiles(dir, &chrome.config.extension_id))
        .collect();
    let installed_version = super::addon::installed_version(
        &profiles,
        &chrome.config.extension_id,
        pairing == "Paired",
        &chrome
            .owner
            .connected_profiles
            .snapshot()
            .into_iter()
            .filter(|(id, _)| super::agent_browser::Browser::Chrome.owns_profile(id))
            .collect::<Vec<_>>(),
    );
    let paired_once = chrome.paired_once.load(std::sync::atomic::Ordering::SeqCst);
    let facts = super::addon::Facts {
        package: setup.package,
        package_text: setup.pairing.clone(),
        set_up: setup.state != setup::SetupState::NotSetUp
            && setup.state != setup::SetupState::Absent,
        browser_issue: setup.browser_issue,
        enabled: chrome.enabled(),
        connected: pairing == "Paired",
        store_review: setup.store_review,
        other_owner: pairing.contains("another Vibe Work"),
        connecting: connecting || pairing == "Connecting",
        waiting,
        helper_up: !chrome
            .owner
            .tools
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        paired_once,
        // Only a disconnected remembered pairing needs an OS probe; live tools avoid a process per read.
        chrome_running: paired_once
            && pairing != "Paired"
            && chrome.enabled()
            && chrome.config.launcher.is_none()
            && super::browser::running(),
        install_state: setup.install_state.clone(),
        extension_folder: !profiles.is_empty(),
        installed_version,
        min_version: shared
            .config
            .vibe_install
            .as_ref()
            .and_then(vw_vibe_install::VibeInstaller::extension_min_version),
    };
    let state = super::addon::state(&facts, || {
        chrome.prober.probe(
            profiles.clone(),
            &chrome.config.extension_id,
            std::time::Instant::now(),
        )
    });
    let mut view = super::addon::view(state, &facts.package_text);
    view.extension_version =
        super::installation::version(shared, super::agent_browser::Browser::Chrome);
    super::addon::reconnecting(&mut view, paired_once && !waiting);
    view
}

/// After Remove: the extension is still in a profile of the folders Remove recorded.
fn extension_left(chrome: &super::ChromeControl) -> bool {
    chrome
        .left_in_chrome
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|dir| !super::probe::profiles(dir, &chrome.config.extension_id).is_empty())
}

impl Core {
    /// Settings > Developer > Local network pages for the agent in Chrome.
    async fn set_local_network(&self, enabled: bool) -> Result<CommandResult, CoreError> {
        self.shared
            .chrome_control
            .set_local_network(enabled)
            .map_err(|reason| invalid(&reason))?;
        if !enabled && let Some(hub) = &self.shared.preview {
            for label in hub.service.mount_labels() {
                if let Some(page) = hub.service.page_for_mount(&label)
                    && hub.browser.on_local_network(&page)
                    && let Some(owner) = hub.service.owner_session_id(&page)
                    && crate::browser_tabs::agent_page_allowed(&hub.service, &owner, &page)
                {
                    hub.service
                        .suspend_page(&page)
                        .await
                        .map_err(|error| invalid(&error.to_string()))?;
                }
            }
        }
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
            offer: offer.map(Box::new),
        }
    }

    fn setup_offer(&self, edge: bool) -> Result<ToolsInstallOffer, CoreError> {
        if edge && !cfg!(feature = "your-edge") {
            return Err(invalid(setup::absent_for(true)));
        }
        let config = &self.shared.chrome_control.config;
        if edge && let Some(reason) = super::edge_policy::refusal(config) {
            return Err(invalid(reason));
        }
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
            OfferKey::new(OFFER_KIND, setup::PLUGIN, if edge { "edge" } else { "" }),
            ToolsInstallOffer {
                offer_id: String::new(),
                kind: OFFER_KIND.to_owned(),
                name: if edge {
                    vw_vocabulary::texts::EDGE_NAME.to_owned()
                } else {
                    vw_vocabulary::texts::CHROME_NAME.to_owned()
                },
                publisher: Some("Vibe Work".to_owned()),
                source: "Shipped with Vibe Work".to_owned(),
                licence: None,
                // The store and Chrome show the extension under its listing name, not "Your Chrome".
                runs: format!(
                    "Adds a small program to Vibe Work's folder, for your user only (no \
                     administrator), and tells {} where it is.{store}",
                    if cfg!(feature = "your-edge") && super::edge_policy::refusal(config).is_none()
                    {
                        "Chrome and Edge"
                    } else {
                        "Google Chrome"
                    }
                ),
                version: None,
                expires_at: 0,
            },
            self.shared.asks.now_ms().into(),
        )
    }

    /// `chromeControl` `setup`: takes the one-time offer, stores the owner identity and installs the helper.
    async fn chrome_setup(
        &self,
        origin: Origin,
        offer_id: String,
        edge: bool,
    ) -> Result<CommandResult, CoreError> {
        if edge && !cfg!(feature = "your-edge") {
            return Err(invalid(setup::absent_for(true)));
        }
        let chrome = &self.shared.chrome_control;
        if edge && let Some(reason) = super::edge_policy::refusal(&chrome.config) {
            return Err(invalid(reason));
        }
        let _held = chrome.lifecycle.lock().await;
        let installed = chrome
            .dir()
            .is_some_and(|dir| dir.join("setup.json").is_file());
        if installed && !edge {
            return Err(invalid(vw_vocabulary::texts::CHROME_ALREADY_SET_UP));
        }
        self.shared.install_offers.take(
            &offer_id,
            &OfferKey::new(OFFER_KIND, setup::PLUGIN, if edge { "edge" } else { "" }),
            self.shared.asks.now_ms().into(),
        )?;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NO_APP_FOLDER))?;
        if installed {
            setup::register_edge(&chrome.config, state).map_err(|reason| invalid(&reason))?;
            if !store_review(&self.shared) {
                super::enroll::connect_edge(&self.shared, origin)
                    .await
                    .map_err(|reason| invalid(&reason))?;
            }
            return Ok(self.chrome_result(None));
        }
        let keys = self
            .shared
            .config
            .keys
            .as_ref()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NEEDS_KEYRING))?;
        keys.create_chrome_owner()
            .await
            .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_STORE_FAILED))?;
        if let Err(reason) =
            setup::install(&chrome.config, state, &self.shared.instructions.own_home)
        {
            let _ = keys.delete_chrome_owner().await;
            return Err(CoreError::new(ErrorCode::Internal, {
                tracing::warn!(%reason, "Chrome Control setup failed");
                "Could not write to Vibe Work’s folder. Nothing was changed. Check free disk space and try again."
            }));
        }
        let browser = if edge {
            super::enroll_browser::Browser::Edge
        } else {
            super::enroll_browser::Browser::Chrome
        };
        let _ = super::enroll::begin(&self.shared, origin, browser).await;
        browser
            .requested(&self.shared)
            .store(true, std::sync::atomic::Ordering::SeqCst);
        chrome
            .set_enabled(true)
            .map_err(|reason| invalid(&reason))?;
        let launched = if store_review(&self.shared) {
            Ok(())
        } else if edge {
            setup::open_store_for(&chrome.config, true)
        } else {
            super::extension_install::begin(&chrome.config, state)
        };
        // The Store opens first; a pending native handshake can then bring the pairing page forward.
        super::start_owner(&self.shared).await;
        tracing::info!("Chrome Control set up");
        if let Err(problem) = launched {
            super::owner::publish(&self.shared);
            return Err(CoreError::new(ErrorCode::Internal, problem));
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
        let user_data = setup::user_data(Some(state), "Google Chrome");
        if let Some(keys) = &self.shared.config.keys {
            keys.delete_chrome_owner()
                .await
                .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_DELETE_FAILED))?;
        }
        // Chrome removes an extension it installed from Vibe Work's registration by itself once the registration
        // is gone; a Store-added extension stays until the person removes it in Chrome.
        let chrome_removes_it = super::extension_install::remove(&chrome.config, state)
            .map_err(|reason| invalid(&reason))?;
        chrome.forget_paired();
        super::edge_state::remember_pairing(chrome, false);
        let removed = setup::remove(&chrome.config, state);
        chrome.reset();
        chrome
            .removed_external
            .store(chrome_removes_it, std::sync::atomic::Ordering::SeqCst);
        *chrome
            .left_in_chrome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = user_data;
        removed.map_err(|reason| {
            CoreError::new(ErrorCode::Internal, {
                tracing::warn!(%reason, "Chrome Control removal failed");
                vw_vocabulary::texts::CHROME_REMOVAL_FAILED
            })
        })?;
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }

    async fn connect_chrome(&self, origin: Origin) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        if store_review(&self.shared) {
            return Err(invalid("Extension in review"));
        }
        let state = chrome
            .dir()
            .filter(|dir| dir.join("setup.json").is_file())
            .ok_or_else(|| invalid("Connect Chrome first"))?;
        if let Some(issue) = super::browser::issue(&chrome.config) {
            return Err(invalid(issue.label()));
        }
        // Chrome may start the helper as soon as it sees the extension: give it this version's copy first.
        super::refresh_helper_held(chrome).await;
        setup::repair_registrations(&chrome.config, state, Some("Google Chrome"))
            .map_err(|_| invalid("Helper setup required"))?;
        chrome
            .set_enabled(true)
            .map_err(|reason| invalid(&reason))?;
        // Only the window's own Connect may mint a pairing ticket; it also revokes older ones.
        let attempt =
            super::enroll::begin(&self.shared, origin, super::enroll_browser::Browser::Chrome)
                .await;
        chrome
            .connect_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // The pairing page opens at once only for a waiting extension the helper reported (or another
        // owner); a helper status answer alone always names our fingerprint and proves no extension.
        let step = self.chrome_view().setup.addon.map(|addon| addon.state);
        let launched = if matches!(
            step,
            Some(super::AddonState::NotPaired | super::AddonState::OtherOwner)
        ) {
            chrome
                .connect_requested
                .store(false, std::sync::atomic::Ordering::SeqCst);
            super::enroll::open_for_waiting(
                &self.shared,
                attempt.as_deref(),
                super::enroll_browser::Browser::Chrome,
            )
            .await;
            Ok(())
        } else {
            super::extension_install::begin(&chrome.config, state)
        };
        super::start_owner(&self.shared).await;
        launched.map_err(|reason| invalid(&reason))?;
        super::owner::publish(&self.shared);
        Ok(self.chrome_result(None))
    }

    /// `chromeControl`.
    ///
    /// # Errors
    /// `invalidParams`/`staleOffer` for a used or expired setup offer; `internal` when
    /// saving availability, setup or removal fails.
    pub(crate) async fn chrome_control(
        &self,
        origin: Origin,
        request: ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let edge = matches!(
            request,
            ChromeControlRequest::OfferEdgeSetup {}
                | ChromeControlRequest::SetupEdge { .. }
                | ChromeControlRequest::OpenEdgeStore {}
        );
        if edge && (!cfg!(feature = "your-edge") || !chrome.config.available()) {
            return Err(invalid(setup::absent_for(true)));
        }
        let firefox = matches!(
            request,
            ChromeControlRequest::OfferFirefoxSetup {}
                | ChromeControlRequest::SetupFirefox { .. }
                | ChromeControlRequest::OpenFirefoxStore {}
                | ChromeControlRequest::OpenFirefox {}
                | ChromeControlRequest::CloseFirefox {}
        );
        if firefox && (!cfg!(feature = "your-firefox") || !chrome.config.available()) {
            return Err(invalid(super::firefox_commands::ABSENT));
        }
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
                let offer = self.setup_offer(false)?;
                Ok(self.chrome_result(Some(offer)))
            }
            ChromeControlRequest::Setup { offer_id } => {
                self.chrome_setup(origin, offer_id, false).await
            }
            ChromeControlRequest::Connect {} => self.connect_chrome(origin).await,
            ChromeControlRequest::OpenStore {} => {
                if store_review(&self.shared) {
                    return Err(invalid("Extension in review"));
                }
                setup::open_store(&chrome.config)
                    .map_err(|reason| CoreError::new(ErrorCode::Internal, reason))?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::OfferEdgeSetup {} => {
                Ok(self.chrome_result(Some(self.setup_offer(true)?)))
            }
            ChromeControlRequest::SetupEdge { offer_id } => {
                self.chrome_setup(origin, offer_id, true).await
            }
            ChromeControlRequest::OpenEdgeStore {} => {
                let _held = chrome.lifecycle.lock().await;
                super::enroll::connect_edge(&self.shared, origin)
                    .await
                    .map_err(|reason| invalid(&reason))?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::OfferFirefoxSetup {}
            | ChromeControlRequest::SetupFirefox { .. }
            | ChromeControlRequest::OpenFirefoxStore {} => Err(invalid(
                "Firefox uses its own window. Choose Open window in Settings.",
            )),
            ChromeControlRequest::OpenFirefox {} => self.firefox_window(true).await,
            ChromeControlRequest::CloseFirefox {} => self.firefox_window(false).await,
            ChromeControlRequest::Remove {} => self.chrome_remove().await,
        }
    }
}
