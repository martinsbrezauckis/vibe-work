//! `chromeControl`: Settings › Browser › Your Chrome and the Chrome Control cards, for the window and the Debug API
//! (both act as the user; the agent has no route here).
//!
//! - `read`: the level and the setup status;
//! - `setLevel`: lowering saves at once; raising asks the native confirmation. The owner Debug API's request
//!   answers its confirmation; a window request still waits for its answer. Cancel keeps the level;
//! - `answer`: the user's answer on a Chrome Control card;
//! - `offerSetup` / `setup`: the one-click set up: the window shows the
//!   offer, the click confirms it, installs and turns Chrome Control's tools on (no second confirmation);
//! - `openStore`: the store page in Chrome; `remove`: deletes what the set up made.
//!
//! Depends on: `chrome_control` (`check`, `setup`, `text`), the native confirmer, the install offers.
//! Called by: `dispatch`.
use serde_json::json;
use vw_ask::chrome::Level;
use vw_ask::{AnsweredBy, Choice, ChromeSettings};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use super::{check, setup, text};
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
    /// The level and the setup status.
    Read {},
    /// Choose the level.
    SetLevel {
        /// The level.
        level: Level,
    },
    /// Answer an open Chrome Control card.
    Answer {
        /// The card's ask id.
        ask_id: String,
        /// once, session or no.
        answer: Choice,
        /// Optional note sent with No.
        #[serde(default)]
        note: Option<String>,
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
    /// The level.
    pub level: Level,
    /// The extension lowered at least one permission; reselecting a level asks before restoring it.
    pub lowered: bool,
    /// Where the setup stands.
    pub setup: setup::ChromeSetupView,
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}

fn not_changed() -> CoreError {
    CoreError::new(ErrorCode::ConfirmationDeclined, "Not changed.")
}

const OFFER_KIND: &str = "chromeControl";

impl Core {
    pub(super) fn chrome_view(&self) -> ChromeControlView {
        let chrome = &self.shared.chrome_control;
        let mut setup = setup::view(
            &chrome.config,
            chrome.dir(),
            &self.shared.instructions.own_home,
        );
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
        ChromeControlView {
            level: chrome.settings().level,
            lowered: !chrome
                .lowered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            setup,
        }
    }

    fn chrome_result(&self, offer: Option<ToolsInstallOffer>) -> CommandResult {
        CommandResult::ChromeControl {
            view: self.chrome_view(),
            offer,
        }
    }

    /// Saves `next`, asking the native confirmation first when it raises the level.
    async fn chrome_change(&self, origin: Origin, next: ChromeSettings) -> Result<(), CoreError> {
        let chrome = &self.shared.chrome_control;
        let now = chrome.settings();
        let restrictions = chrome
            .lowered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let next_policy = super::mapping::policy(
            next,
            crate::browser_delegation::load(&self.shared).close_user_tabs,
        );
        let rank = |value: &str| match value {
            "allow" => 2,
            "ask" => 1,
            _ => 0,
        };
        let restores = restrictions.iter().any(|(cap, value)| {
            rank(next_policy["policy"][cap].as_str().unwrap_or("never")) > rank(value)
        });
        if now.raises(next) || restores {
            let request = ConfirmationRequest {
                method: vw_ask::chrome::LOOSEN_KIND.to_owned(),
                summary: text::raise_summary(now, next),
                params: json!({"level": next.level}),
                requester: crate::command::requester(origin),
                always: None,
            };
            if self.shared.confirmer.confirm(&request).await != Confirmation::Approved {
                return Err(not_changed());
            }
            // Another change landed while the dialog was open: the user confirmed a change from another level.
            if chrome.settings() != now {
                return Err(not_changed());
            }
        }
        let _held = chrome.lifecycle.lock().await;
        if chrome.settings() != now
            || *chrome
                .lowered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                != restrictions
        {
            return Err(not_changed());
        }
        chrome.save(next).map_err(|reason| {
            CoreError::new(
                ErrorCode::Internal,
                vw_vocabulary::text("chromeSaveFailed")
                    .unwrap_or("Could not save the Chrome Control level: {reason}.")
                    .replace("{reason}", &reason),
            )
        })?;
        if let Some(keys) = &self.shared.config.keys {
            keys.save_chrome_restrictions(std::collections::BTreeMap::new())
                .await
                .map_err(|_| invalid("Could not save Chrome Control's permissions."))?;
        }
        chrome
            .lowered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        tracing::info!(level = next.level.id(), "Chrome Control level saved");
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(())
    }

    fn setup_offer(&self) -> Result<ToolsInstallOffer, CoreError> {
        let config = &self.shared.chrome_control.config;
        setup::packaged_helper(config).map_err(|reason| invalid(&reason))?;
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
                     administrator), tells Google Chrome where it is, and makes Chrome Control available when you allow it in a chat. Opens the Chrome Web Store page so you can add the extension, {}.",
                    setup::STORE_LISTING_NAME
                ),
                version: None,
                expires_at: 0,
            },
            self.shared.asks.now_ms(),
        )
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
        super::removed(&self.shared);
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
    /// `confirmationDeclined` ("Not changed.") when a raise was not confirmed; `invalidCommand` for a closed
    /// card or an answer not offered; `invalidParams`/`staleOffer` for a used or expired setup offer; `internal` when
    /// a save, the set up or the remove failed (nothing changed).
    pub(crate) async fn chrome_control(
        &self,
        origin: Origin,
        request: ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        if !chrome.config.available() && !matches!(request, ChromeControlRequest::Read {}) {
            return Err(invalid(setup::ABSENT));
        }
        match request {
            ChromeControlRequest::Read {} => Ok(self.chrome_result(None)),
            ChromeControlRequest::SetLevel { level } => {
                self.chrome_change(origin, ChromeSettings { level }).await?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::Answer {
                ask_id,
                answer,
                note,
            } => {
                let by = if crate::command_origin::debug_api_acts(&self.shared, origin) {
                    AnsweredBy::DebugApi
                } else {
                    AnsweredBy::Person
                };
                check::answer(&self.shared, &ask_id, answer, note, by).await?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::OfferSetup {} => {
                let offer = self.setup_offer()?;
                Ok(self.chrome_result(Some(offer)))
            }
            ChromeControlRequest::Setup { offer_id } => {
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
                if let Err(problem) = setup::open_store(&chrome.config) {
                    tracing::warn!(%problem, "the Chrome Web Store page could not be opened");
                }
                self.shared.emit(crate::CoreEvent::ChromeControl {
                    view: self.chrome_view(),
                });
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::OpenStore {} => {
                setup::open_store(&chrome.config)
                    .map_err(|reason| CoreError::new(ErrorCode::Internal, reason))?;
                Ok(self.chrome_result(None))
            }
            ChromeControlRequest::Remove {} => self.chrome_remove().await,
        }
    }
}
