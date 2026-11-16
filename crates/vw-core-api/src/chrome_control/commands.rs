//! Browser availability and connection setup for Settings and the Debug API.
//! Called by core dispatch; permissions come from chat access modes.

use super::{agent_browser::Browser, setup};
use crate::install_offers::{OfferKey, ToolsInstallOffer};
use crate::{CommandResult, Core, CoreError, ErrorCode, Origin};
use vw_platform::sync::LockRecover;

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
    /// Select this chat's exact profile after the old task acknowledges release.
    SelectProfile {
        /// Owning chat or New chat draft.
        session_id: String,
        /// Exact paired profile identity.
        profile_id: String,
    },
    /// Choose this browser's default for new chat bindings.
    SetDefaultProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Ask in each chat again: clear this browser's default when it is still this identity.
    ClearDefaultProfile {
        /// The current default's exact paired identity.
        profile_id: String,
    },
    /// Enable or fence only this profile.
    SetProfileEnabled {
        /// Exact paired identity.
        profile_id: String,
        /// Whether the profile is available.
        enabled: bool,
    },
    /// Reconnect a previously paired identity without choosing another account.
    ConnectProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Remove this app pairing, leaving browser data and sign-ins intact.
    RemoveProfile {
        /// Exact paired identity.
        profile_id: String,
    },
    /// Claim one person-shared browser context item for the composer.
    TakeSelection {
        /// The pending selection identity.
        selection_id: String,
    },
    /// The per-chat agent control frame colour (kept across restarts, the same in every browser that frames the
    /// chat's work). Every chat starts at the one default colour, index 0.
    SetFrameColour {
        /// The chat.
        session_id: String,
        /// Palette index 0-7.
        colour: u8,
    },
    /// List connected tabs without opening a chat window.
    Tabs {},
    /// Read the exact tab selected by the person.
    TabContext {
        /// Connected profile identity.
        profile_id: String,
        /// Exact tab identity.
        tab_id: u32,
        /// Receiving chat or New chat's draft owner; omitted for a read-only context lookup.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Turn Chrome tools on or off.
    SetEnabled {
        /// Whether Chrome tools are available.
        enabled: bool,
    },
    /// Turn Edge tools on or off without changing Chrome.
    SetEdgeEnabled {
        /// Whether Edge tools are available.
        enabled: bool,
    },
    /// Turn Firefox tools on or off without changing the other browsers.
    SetFirefoxEnabled {
        /// Whether Firefox tools are available.
        enabled: bool,
    },
    /// Close Firefox and remove its connection.
    RemoveFirefox {},
    /// Legacy command: refreshes the enabled compatibility policy sent to older extensions.
    SetLocalNetwork {
        /// Ignored legacy value; browser address access is always enabled.
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
    /// Open Safari setup through the utility embedded in Vibe Work.
    OpenSafariSetup {},
    /// The person's independent Safari availability switch.
    SetSafariEnabled {
        /// Whether the optional Safari connection is enabled.
        enabled: bool,
    },
    /// Unregister only the embedded Safari connection and leave it off.
    RemoveSafari {},
    /// Remove only the Chrome connection.
    Remove {},
    /// Remove only the Edge connection.
    RemoveEdge {},
    /// macOS: read browser folders the system refused again, after the person may have allowed them.
    RecheckProfileNames {},
    /// macOS: open System Settings where the person allows Vibe Work to read Chrome's or Edge's profile names.
    OpenProfileNameSettings {},
}

/// Settings › Browser › Your Chrome.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChromeControlView {
    /// Composer visibility and built-in availability, independent of paired-profile Off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub browser_preferences: Option<crate::BrowserPreferences>,
    /// Whether Chrome tools are available.
    pub enabled: bool,
    /// Edge's independent switch, omitted while its module is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub edge_enabled: Option<bool>,
    /// Legacy compatibility status; browser address access is always enabled.
    pub local_network: bool,
    /// The running helper acknowledged `local_network`'s current value (false while a change is still being sent).
    pub local_network_applied: bool,
    /// Recent chat work, bounded and independent of permissions.
    pub work: Vec<super::ChromeWorkView>,
    /// Individually paired identities, including retained offline profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub paired_profiles: Option<Vec<super::BrowserProfile>>,
    /// Fixed chat selections, including unconfirmed switches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub chat_profiles: Option<Vec<super::ChatBrowserProfile>>,
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
    /// Firefox availability, independent of Chrome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub firefox_enabled: Option<bool>,
    /// The optional person-share add-on, independent of the owned window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub firefox_sharing: Option<super::AddonView>,
    /// Safari is omitted while its optional module is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub safari: Option<super::SafariView>,
    /// Chats whose agent control frame has a colour other than the default (chat id -> palette index 0-7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub frame_colours: Option<Box<std::collections::BTreeMap<String, u8>>>,
}

pub(super) fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}

const OFFER_KIND: &str = "chromeControl";

#[path = "commands_setup.rs"]
mod setup_commands;

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

pub(crate) use super::commands_view::{addon, view};

impl Core {
    /// Refreshes the legacy extension compatibility policy.
    async fn set_local_network(&self, enabled: bool) -> Result<CommandResult, CoreError> {
        self.shared.chrome_control.set_local_network(enabled);
        // Older extensions acknowledge compatibility delivery before new actions.
        super::send_local_network(&self.shared).await;
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }

    pub(super) fn chrome_view(&self) -> ChromeControlView {
        view(&self.shared)
    }

    /// Connect while a profile waits after "Not now": ask it again instead of setting up (`pairing_approval`).
    async fn ask_again(&self, origin: Origin, browser: Browser) -> Option<CommandResult> {
        let hint = super::pairing_approval::ask_again(&self.shared, origin, browser).await?;
        Some(CommandResult::ChromeControl {
            view: self.chrome_view(),
            offer: None,
            hint: Some(hint),
        })
    }

    fn chrome_result(&self, offer: Option<ToolsInstallOffer>) -> CommandResult {
        CommandResult::ChromeControl {
            view: self.chrome_view(),
            offer: offer.map(Box::new),
            hint: None,
        }
    }

    /// macOS profile-name access: read refused browser folders again, or open System Settings where it is allowed.
    async fn profile_names_access(
        &self,
        request: &ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        self.check_browser_request(request)?;
        if *request == (ChromeControlRequest::RecheckProfileNames {}) {
            let shared = self.shared.clone();
            tokio::task::spawn_blocking(move || super::browser_data::recheck(&shared))
                .await
                .map_err(|_| CoreError::new(ErrorCode::Internal, "Recheck stopped"))?;
        } else {
            let config = self.shared.chrome_control.config.clone();
            tokio::task::spawn_blocking(move || super::browser_data::open_settings(&config))
                .await
                .map_err(|_| CoreError::new(ErrorCode::Internal, "System Settings did not open"))?
                .map_err(|reason| invalid(&reason))?;
        }
        Ok(self.chrome_result(None))
    }

    fn chrome_open_store(&self) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        if chrome
            .owner
            .credential_missing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(invalid(vw_vocabulary::texts::CHROME_OWNER_MISSING));
        }
        if chrome
            .owner
            .keyring_unavailable
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(invalid(
                &Browser::Chrome.words(vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE),
            ));
        }
        if store_review(&self.shared) {
            return Err(invalid("Extension in review"));
        }
        setup::open_store(&chrome.config)
            .map_err(|reason| CoreError::new(ErrorCode::Internal, reason))?;
        Ok(self.chrome_result(None))
    }

    async fn connect_chrome(&self, origin: Origin) -> Result<CommandResult, CoreError> {
        if let Some(asked) = self.ask_again(origin, Browser::Chrome).await {
            return Ok(asked);
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        if chrome
            .owner
            .credential_missing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(invalid(vw_vocabulary::texts::CHROME_OWNER_MISSING));
        }
        if let Some(sentence) = super::owner::keyring_blocked(&self.shared).await {
            return Err(invalid(&Browser::Chrome.words(sentence)));
        }
        if store_review(&self.shared) {
            return Err(invalid("Extension in review"));
        }
        setup::packaged_helper(&chrome.config).map_err(|reason| invalid(&reason))?;
        let state = chrome
            .dir()
            .filter(|dir| dir.join("setup.json").is_file())
            .ok_or_else(|| invalid("Connect Chrome first"))?;
        super::browser::refresh(&chrome.config).await;
        if let Some(issue) = super::browser::issue(&chrome.config) {
            return Err(invalid(issue.label()));
        }
        // Chrome may start the helper as soon as it sees the extension: give it this version's copy first.
        super::refresh_helper_held(chrome).await;
        setup::repair_registrations(&chrome.config, state, Some("Google Chrome"))
            .map_err(|reason| invalid(&reason))?;
        chrome
            .set_enabled(true)
            .map_err(|reason| invalid(&reason))?;
        super::switches::deliver(&self.shared).await?;
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
            super::extension_install::begin_async(&chrome.config, state).await
        };
        super::start_owner_for(&self.shared, Some("Google Chrome")).await;
        if chrome
            .owner
            .credential_missing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(invalid(vw_vocabulary::texts::CHROME_OWNER_MISSING));
        }
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
        let result = match request {
            request @ (ChromeControlRequest::RecheckProfileNames {}
            | ChromeControlRequest::OpenProfileNameSettings {}) => {
                self.profile_names_access(&request).await
            }
            // The built-in browser frames chats too: the colour needs no Chrome setup.
            ChromeControlRequest::SetFrameColour { session_id, colour } => {
                super::frames::set_colour(&self.shared, &session_id, colour).map(|()| {
                    super::owner::publish(&self.shared);
                    self.chrome_result(None)
                })
            }
            request => self.chrome_control_inner(origin, request).await,
        };
        if result.is_err() {
            super::owner::publish(&self.shared);
        }
        result
    }

    async fn chrome_control_inner(
        &self,
        origin: Origin,
        request: ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        self.check_browser_request(&request)?;
        match request {
            ChromeControlRequest::Read {} => {
                super::browser::refresh(&chrome.config).await;
                super::safari_copies::refresh(&self.shared).await;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::SelectProfile {
                session_id,
                profile_id,
            } => super::profile_selection::command(self, origin, &session_id, &profile_id).await,
            request if super::profile_commands::is_setting(&request) => {
                self.profile_setting(origin, request).await
            }
            ChromeControlRequest::OpenSafariSetup {} => self.safari_setup().await,
            ChromeControlRequest::RemoveSafari {} => self.safari_remove().await,
            ChromeControlRequest::SetSafariEnabled { enabled } => {
                self.safari_enabled(enabled).await
            }
            ChromeControlRequest::TakeSelection { selection_id } => {
                self.chrome_selection(&selection_id).await
            }
            ChromeControlRequest::Tabs {} => super::tabs::read(&self.shared, chrome).await,
            ChromeControlRequest::TabContext {
                profile_id,
                tab_id,
                session_id,
            } => {
                if let Some(chat) = session_id.as_deref()
                    && crate::command_origin::debug_api_acts(&self.shared, origin)
                    && !super::profile_bindings::selected(chrome, chat)
                        .is_some_and(|old| super::profile_ids::same(&old, &profile_id))
                {
                    return Err(invalid(super::profile_bindings::tab_refusal(chrome, chat)));
                }
                super::tabs::pick(
                    &self.shared,
                    chrome,
                    &profile_id,
                    tab_id,
                    session_id.as_deref(),
                )
                .await
            }
            ChromeControlRequest::SetEnabled { enabled } => {
                self.browser_enabled(Browser::Chrome, enabled).await
            }
            ChromeControlRequest::SetEdgeEnabled { enabled } => {
                if !cfg!(feature = "your-edge") {
                    return Err(invalid(setup::absent_for(true)));
                }
                self.browser_enabled(Browser::Edge, enabled).await
            }
            ChromeControlRequest::SetLocalNetwork { enabled } => {
                self.set_local_network(enabled).await
            }
            ChromeControlRequest::OfferSetup {} => {
                Ok(self.chrome_result(Some(self.setup_offer(false)?)))
            }
            ChromeControlRequest::Setup { offer_id } => {
                self.chrome_setup(origin, offer_id, false).await
            }
            ChromeControlRequest::Connect {} => self.connect_chrome(origin).await,
            ChromeControlRequest::OpenStore {} => self.chrome_open_store(),
            ChromeControlRequest::OfferEdgeSetup {} => {
                Ok(self.chrome_result(Some(self.setup_offer(true)?)))
            }
            ChromeControlRequest::SetupEdge { offer_id } => {
                self.chrome_setup(origin, offer_id, true).await
            }
            ChromeControlRequest::OpenEdgeStore {} => {
                if let Some(asked) = self.ask_again(origin, Browser::Edge).await {
                    return Ok(asked);
                }
                let _held = chrome.lifecycle.lock().await;
                super::enroll::connect_edge(&self.shared, origin)
                    .await
                    .map_err(|reason| invalid(&reason))?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::OfferFirefoxSetup {} => {
                Ok(self.chrome_result(Some(self.firefox_setup_offer()?)))
            }
            ChromeControlRequest::SetupFirefox { offer_id } => self.firefox_setup(offer_id).await,
            ChromeControlRequest::OpenFirefoxStore {} => self.firefox_addons(),
            ChromeControlRequest::SetFirefoxEnabled { enabled } => {
                self.browser_enabled(Browser::Firefox, enabled).await
            }
            ChromeControlRequest::RemoveFirefox {} => self.firefox_remove().await,
            ChromeControlRequest::OpenFirefox {} => self.firefox_window(true).await,
            ChromeControlRequest::CloseFirefox {} => self.firefox_window(false).await,
            ChromeControlRequest::Remove {} => self.remove_browser(Browser::Chrome).await,
            ChromeControlRequest::RemoveEdge {} => self.remove_browser(Browser::Edge).await,
            // Recheck and System Settings: `chrome_control` sends them to `profile_names_access` before this match.
            request => self.profile_names_access(&request).await,
        }
    }

    fn check_browser_request(&self, request: &ChromeControlRequest) -> Result<(), CoreError> {
        let chrome = &self.shared.chrome_control;
        let edge = matches!(
            &request,
            ChromeControlRequest::RemoveEdge {}
                | ChromeControlRequest::OfferEdgeSetup {}
                | ChromeControlRequest::SetupEdge { .. }
                | ChromeControlRequest::OpenEdgeStore {}
        );
        if edge && (!cfg!(feature = "your-edge") || !chrome.config.available()) {
            return Err(invalid(setup::absent_for(true)));
        }
        let firefox = matches!(
            &request,
            ChromeControlRequest::OfferFirefoxSetup {}
                | ChromeControlRequest::SetupFirefox { .. }
                | ChromeControlRequest::OpenFirefoxStore {}
                | ChromeControlRequest::OpenFirefox {}
                | ChromeControlRequest::CloseFirefox {}
                | ChromeControlRequest::SetFirefoxEnabled { .. }
                | ChromeControlRequest::RemoveFirefox {}
        );
        if firefox && (!cfg!(feature = "your-firefox") || !chrome.config.available()) {
            return Err(invalid(super::firefox_commands::ABSENT));
        }
        if !chrome.config.available()
            && !matches!(
                &request,
                ChromeControlRequest::Read {} | ChromeControlRequest::TakeSelection { .. }
            )
        {
            return Err(invalid(setup::ABSENT));
        }
        Ok(())
    }

    async fn chrome_selection(&self, selection_id: &str) -> Result<CommandResult, CoreError> {
        let pending = self
            .shared
            .chrome_control
            .selections
            .lock_or_recover()
            .take(selection_id);
        super::owner::publish(&self.shared);
        let selection = if let Some(pending) = pending {
            let result = super::sharing::claim(&self.shared, pending).await;
            if result.is_err() {
                self.shared
                    .chrome_control
                    .selections
                    .lock_or_recover()
                    .error = Some(
                    "The shared page could not be read. Choose Ask Vibe Work on that page again."
                        .into(),
                );
                super::owner::publish(&self.shared);
            }
            Some(result?)
        } else {
            None
        };
        Ok(CommandResult::ChromeSelection { selection })
    }
}
