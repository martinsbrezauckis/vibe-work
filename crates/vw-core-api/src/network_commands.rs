//! Reads, tests and saves proxy/certificate settings through the existing core command path.
//! Uses network's pure field validation/projection and the key service; called by Core dispatch.
//! Persisted settings and Vibe read-back retain their existing order and refusal behavior.
use super::{after_write, check_changes, overridden, rejection_notice, view_of, writes};
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::network_view::{NetworkChanges, NetworkProblem, NetworkRequest, NetworkResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vw_mistral::network::{self as net, NetworkSettings, PreparedSettings, has_sign_in, scrub};

impl Core {
    /// Executes one `network` request.
    ///
    /// # Errors
    /// `denied` (not in this build) when a save is asked in a run that has no place for the core's file;
    /// `serverUnavailable` when Vibe is not running (a save writes to Vibe too); `confirmationDeclined` when the user
    /// declined the dialog; Vibe's own error for a write it refuses (with the values scrubbed out of its text).
    /// `keyService` when this run cannot replace the protected network-authentication key.
    pub(crate) async fn network(
        &self,
        origin: Origin,
        request: NetworkRequest,
    ) -> Result<NetworkResult, CoreError> {
        match request {
            NetworkRequest::Read {} => Ok(self.network_read().await),
            NetworkRequest::Test { changes } => Ok(self.network_try(&changes).await),
            NetworkRequest::Save { changes } => self.network_save(origin, &changes).await,
        }
    }

    /// What the core keeps: its own file's settings (empty without a file or before the first save).
    async fn saved_settings(&self) -> NetworkSettings {
        let Some(file) = &self.shared.config.network_file else {
            return NetworkSettings::default();
        };
        let loaded = if let Some(keys) = &self.shared.config.keys {
            keys.read_network_settings(file.clone()).await
        } else {
            net::read_settings(file, None)
        };
        if loaded.rejected
            && !self
                .shared
                .network_notice
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            self.shared
                .emit(rejection_notice(self.shared.config.keys.as_deref()));
        }
        loaded.settings
    }

    /// Writes the core's own file, user-only, atomically.
    async fn persist(&self, prepared: &PreparedSettings) -> Result<(), CoreError> {
        let file = self
            .shared
            .config
            .network_file
            .as_deref()
            .ok_or_else(no_file)?;
        let keys = self.shared.config.keys.as_ref().ok_or_else(no_keys)?;
        let text = keys
            .seal_network_settings(prepared.clone())
            .await
            .map_err(|error| CoreError::new(ErrorCode::KeyService, error.to_string()))?;
        let file = file.to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            file.parent()
                .map_or(Ok(()), vw_platform::ensure_private_dir)
                .and_then(|()| vw_platform::replace_private_durable(&file, text.as_bytes()))
        })
        .await
        .map_err(|join| join.to_string())
        .and_then(|written| written.map_err(|why| why.to_string()));
        result.map_err(|why| {
            CoreError::new(
                ErrorCode::Internal,
                format!(
                    "Vibe saved the change, but Vibe Work could not keep its own copy ({why}). Save again once that is fixed."
                ),
            )
        })
    }

    pub(crate) async fn network_read(&self) -> NetworkResult {
        NetworkResult::Settings {
            view: Box::new(view_of(
                &self.saved_settings().await,
                &NetworkSettings::from_environment(),
                &net::current(),
            )),
            saved: false,
            leftover_session_id: None,
        }
    }

    async fn network_try(&self, changes: &NetworkChanges) -> NetworkResult {
        let saved = self.saved_settings().await;
        let checked = match check_changes(changes, &saved) {
            Ok(checked) => checked,
            Err(problems) => return NetworkResult::Refused { problems },
        };
        // The user's list as typed; this computer's own addresses are added when the client is built.
        let tried = &checked.settings;
        let environment = NetworkSettings::from_environment();
        let used = NetworkSettings::effective(tried, &environment);
        if net::KEYS[..3]
            .iter()
            .any(|key| used.get(key).is_some_and(|value| has_sign_in(&value)))
            && !self
                .shared
                .config
                .keys
                .as_ref()
                .is_some_and(|keys| keys.proxy_sign_in_ready())
        {
            return NetworkResult::Refused {
                problems: vec![NetworkProblem {
                    field: "httpsProxy".into(),
                    message: "Unlock the vault to use the proxy sign-in.".into(),
                }],
            };
        }
        let forwarded = self
            .shared
            .config
            .keys
            .as_ref()
            .ok_or(vw_keys::KeyError::Internal)
            .and_then(|keys| {
                keys.network_probe(&used, self.shared.config.network_test_resolver.clone())
            });
        let Ok((guard, child)) = forwarded else {
            return NetworkResult::Refused {
                problems: vec![NetworkProblem {
                    field: "httpsProxy".into(),
                    message: "The proxy forwarder could not start.".into(),
                }],
            };
        };
        let protected = net::KEYS[..3]
            .iter()
            .any(|key| used.get(key).is_some_and(|value| has_sign_in(&value)));
        let probe = net::probe(
            if protected { &child } else { &used },
            &self.shared.config.network_test_url,
        )
        .await;
        let mut outcome =
            crate::network_test::describe(probe, &used, overridden(tried, &environment));
        crate::network_python::verify(&self.shared, &child, &mut outcome).await;
        drop(guard);
        NetworkResult::Tested { outcome }
    }

    async fn network_save(
        &self,
        origin: Origin,
        changes: &NetworkChanges,
    ) -> Result<NetworkResult, CoreError> {
        if self.shared.config.network_file.is_none() {
            return Err(no_file());
        }
        if self.shared.config.keys.is_none() {
            return Err(no_keys());
        }
        let saved = self.saved_settings().await;
        let environment = NetworkSettings::from_environment();
        let checked = match check_changes(changes, &saved) {
            Ok(checked) => checked,
            Err(problems) => return Ok(NetworkResult::Refused { problems }),
        };
        let write = writes(&saved, &checked);
        if write.is_empty() {
            return Ok(NetworkResult::Settings {
                view: Box::new(view_of(&saved, &environment, &net::current())),
                saved: false,
                leftover_session_id: None,
            });
        }
        let prepared = net::prepare_settings(&after_write(&checked.settings, &write))
            .map_err(|error| CoreError::new(ErrorCode::InvalidParams, error.to_string()))?;
        // Vibe first (it asks the native dialog and may refuse), through a short-lived chat of ours; the core's own
        // file only after Vibe took the change.
        let id = self
            .start_helper_chat(origin, "The network settings were not saved")
            .await?;
        let written = self.write_to_vibe(origin, &id, &write, &prepared).await;
        let gone = self.end_helper_chat(&id).await;
        let leftover = (!gone).then_some(id);
        if let Err(mut error) = written {
            if leftover.is_some() {
                error.message.push_str(
                    " The helper chat used for it could not be removed and stays in the list until Vibe Work closes.",
                );
            }
            return Err(error);
        }
        let after = prepared.settings();
        self.persist(&prepared).await?;
        if let (Some(keys), Some(file)) =
            (&self.shared.config.keys, &self.shared.config.network_file)
        {
            keys.network_saved(
                file.clone(),
                self.shared.instructions.own_home.join(".env"),
                write.keys().map(|key| (*key).to_owned()).collect(),
            )
            .await
            .map_err(|error| CoreError::new(ErrorCode::KeyService, error.to_string()))?;
        }
        Ok(NetworkResult::Settings {
            view: Box::new(view_of(after, &environment, &net::current())),
            saved: true,
            leftover_session_id: leftover,
        })
    }

    /// `config/proxy/read` through `session`: Vibe's raw values, only to check that a write was kept (they are never
    /// shown: Vibe's `.env` is not a source of the core's settings).
    async fn read_vibe(&self, origin: Origin, session: &str) -> Result<NetworkSettings, CoreError> {
        let reply = self
            .forward(
                origin,
                "config/proxy/read",
                json!({"sessionId": session}),
                None,
            )
            .await?;
        let values = reply
            .result
            .pointer("/settings/values")
            .and_then(Value::as_object)
            .ok_or_else(|| CoreError::bad_result("config/proxy/read", "settings.values"))?;
        Ok(NetworkSettings::from_lookup(|key| {
            values.get(key).and_then(Value::as_str).map(str::to_owned)
        }))
    }

    /// `config/proxy/write` for `write`, confirmed natively, then read back: every key must be as it was sent.
    async fn write_to_vibe(
        &self,
        origin: Origin,
        session: &str,
        write: &BTreeMap<&'static str, Option<String>>,
        prepared: &PreparedSettings,
    ) -> Result<(), CoreError> {
        let keys = self.shared.config.keys.as_ref().ok_or_else(no_keys)?;
        let child = keys.child_network_settings(prepared.settings());
        let child_write: BTreeMap<&str, Option<String>> =
            write.keys().map(|key| (*key, child.get(key))).collect();
        let changes: serde_json::Map<String, Value> = write
            .iter()
            .map(|(key, value)| {
                (
                    (*key).to_owned(),
                    value.as_ref().map_or(Value::Null, |v| json!(v)),
                )
            })
            .collect();
        // Only an address can carry a sign-in; those are what Vibe's error text may repeat.
        let raw: Vec<&str> = write
            .iter()
            .filter(|(key, _)| matches!(**key, "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY"))
            .filter_map(|(_, value)| value.as_deref())
            .collect();
        let original = json!({"sessionId": session, "changes": changes});
        let params = json!({"sessionId": session, "changes": child_write});
        let method = "config/proxy/write";
        let fingerprints = ["SSL_CERT_FILE", "SSL_CERT_DIR"]
            .into_iter()
            .filter(|key| write.contains_key(key))
            .filter_map(|key| {
                let fingerprint = prepared.certificate_fingerprint(key)?;
                let path = prepared.settings().get(key)?;
                let label = if key == "SSL_CERT_FILE" {
                    "Certificate file"
                } else {
                    "Certificate folder"
                };
                Some(format!(
                    "\n{label}: {}\nCertificate fingerprint: {fingerprint}",
                    vw_gate::dialog_quote(&path)
                ))
            })
            .collect::<String>();
        self.shared.dialog_texts.set(
            method,
            &params,
            format!(
                "{}{fingerprints}",
                vw_gate::proxy_confirmation_summary(&original)
            ),
        );
        let scrubbed = |mut error: CoreError| {
            error.message = scrub(&error.message, &raw);
            error.summary = error.summary.map(|s| scrub(&s, &raw));
            error
        };
        let token = self
            .token_for(origin, method, &params)
            .await
            .map_err(|error| {
                let _ = self.shared.dialog_texts.take(method, &params);
                scrubbed(error)
            })?;
        let _ = self.shared.dialog_texts.take(method, &params);
        keys.pause_vibe_proxy(prepared.settings());
        self.forward(origin, method, params, token.map(|t| t.as_str().to_owned()))
            .await
            .map_err(scrubbed)?;
        let after = self.read_vibe(origin, session).await?;
        for (key, wanted) in &child_write {
            if after.get(key).as_deref() != wanted.as_deref() {
                return Err(CoreError::new(
                    ErrorCode::ServerError,
                    format!("Vibe did not keep the change to {key}."),
                )
                .with_method(method));
            }
        }
        Ok(())
    }
}

fn no_file() -> CoreError {
    CoreError::not_in_this_build(
        "This run has no place to keep network settings (no app data folder), so they cannot be saved.",
    )
}

fn no_keys() -> CoreError {
    CoreError::new(
        ErrorCode::KeyService,
        "Network settings could not be saved. Restart Vibe Work and try again.",
    )
}
