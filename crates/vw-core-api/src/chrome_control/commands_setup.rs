//! Browser setup commands for the Settings Connect action. Uses scoped registration and owner enrollment;
//! permission failures keep their diagnosis in the notice and the shared add-on state.
use super::{
    CommandResult, Core, CoreError, ErrorCode, OFFER_KIND, OfferKey, Origin, ToolsInstallOffer,
    invalid, setup, store_review,
};

impl Core {
    pub(super) fn setup_offer(&self, edge: bool) -> Result<ToolsInstallOffer, CoreError> {
        if edge && !cfg!(feature = "your-edge") {
            return Err(invalid(setup::absent_for(true)));
        }
        let config = &self.shared.chrome_control.config;
        if edge && let Some(reason) = super::super::edge_policy::refusal(config) {
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
                    if config.is_macos() {
                        if edge {
                            "Microsoft Edge"
                        } else {
                            "Google Chrome"
                        }
                    } else if cfg!(feature = "your-edge")
                        && super::super::edge_policy::refusal(config).is_none()
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
    pub(super) async fn chrome_setup(
        &self,
        origin: Origin,
        offer_id: String,
        edge: bool,
    ) -> Result<CommandResult, CoreError> {
        if edge && !cfg!(feature = "your-edge") {
            return Err(invalid(setup::absent_for(true)));
        }
        let chrome = &self.shared.chrome_control;
        if edge && let Some(reason) = super::super::edge_policy::refusal(&chrome.config) {
            return Err(invalid(reason));
        }
        let _held = chrome.lifecycle.lock().await;
        let installed = chrome
            .dir()
            .is_some_and(|dir| dir.join("setup.json").is_file());
        if installed
            && !edge
            && (!chrome.config.is_macos()
                || setup::registered(&chrome.config, chrome.dir(), "Google Chrome"))
        {
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
        if installed && edge {
            setup::register_edge(&chrome.config, state).map_err(|reason| invalid(&reason))?;
            if !store_review(&self.shared) {
                super::super::enroll::connect_edge(&self.shared, origin)
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
        if !installed {
            keys.create_chrome_owner()
                .await
                .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_STORE_FAILED))?;
        }
        if let Err(reason) = setup::install_for(
            &chrome.config,
            state,
            &self.shared.instructions.own_home,
            if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            },
        ) {
            if !installed {
                let _ = keys.delete_chrome_owner().await;
            }
            return Err(CoreError::new(ErrorCode::Internal, {
                tracing::warn!(%reason, "Chrome Control setup failed");
                super::super::owner::publish(&self.shared);
                setup_failure_message(&reason)
            }));
        }
        self.finish_browser_setup(origin, edge).await
    }

    async fn finish_browser_setup(
        &self,
        origin: Origin,
        edge: bool,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NO_APP_FOLDER))?;
        let browser = if edge {
            super::super::enroll_browser::Browser::Edge
        } else {
            super::super::enroll_browser::Browser::Chrome
        };
        let _ = super::super::enroll::begin(&self.shared, origin, browser).await;
        browser
            .requested(&self.shared)
            .store(true, std::sync::atomic::Ordering::SeqCst);
        chrome
            .set_browser_enabled(
                if edge {
                    super::super::agent_browser::Browser::Edge
                } else {
                    super::super::agent_browser::Browser::Chrome
                },
                true,
            )
            .map_err(|reason| invalid(&reason))?;
        super::super::switches::deliver(&self.shared).await?;
        let launched = if store_review(&self.shared) {
            Ok(())
        } else if edge {
            setup::open_store_for(&chrome.config, true)
        } else {
            super::super::extension_install::begin_async(&chrome.config, state).await
        };
        // The Store opens first; a pending native handshake can then bring the pairing page forward.
        super::super::start_owner_for(
            &self.shared,
            Some(if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            }),
        )
        .await;
        tracing::info!("Chrome Control set up");
        if let Err(problem) = launched {
            super::super::owner::publish(&self.shared);
            return Err(CoreError::new(ErrorCode::Internal, problem));
        }
        self.shared.emit(crate::CoreEvent::ChromeControl {
            view: self.chrome_view(),
        });
        Ok(self.chrome_result(None))
    }
}

/// Keep the OS diagnosis; a generic write failure never establishes a full disk. A reason that already says what to
/// do ("Open Google Chrome once, then retry Connect.") keeps its own instruction instead of a second one.
fn setup_failure_message(reason: &str) -> String {
    if reason.starts_with("macOS blocked access") {
        return reason.to_owned();
    }
    let reason = reason.trim_end_matches('.');
    if reason.contains("retry Connect") {
        format!("Browser setup failed: {reason}.")
    } else {
        format!("Browser setup failed: {reason}. Try Connect again.")
    }
}

#[cfg(test)]
mod setup_failure_message_tests {
    #[test]
    fn a_reason_with_its_own_instruction_is_not_followed_by_a_second_one() {
        assert_eq!(
            super::setup_failure_message("Open Google Chrome once, then retry Connect."),
            "Browser setup failed: Open Google Chrome once, then retry Connect."
        );
        assert_eq!(
            super::setup_failure_message("the folder could not be written"),
            "Browser setup failed: the folder could not be written. Try Connect again."
        );
    }
}
