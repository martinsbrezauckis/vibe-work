//! The `network` command: Settings > Network. Shows the proxy and certificate settings the core keeps, tests the way to
//! Mistral with them, and saves changes (to Vibe with `config/proxy/write`, and to the core's own file).
//!
//! Why the core and not the window: Vibe's model traffic leaves through the core's own model proxy, so a setting
//! only Vibe knew would not move one request. The core's clients carry the real Mistral key, so what they connect
//! through must be authenticated: the agent can write app data, so the core verifies its file with a random MAC
//! key in the Mistral key's vault (a copy in the computer's credential store checks it while the vault is locked).
//! It never reads Vibe's `.env`. A save writes both: Vibe's `.env` through
//! `config/proxy/write` (for Vibe's own traffic, which holds only the
//! placeholder key) and the core's file. What Settings shows, and what the core's clients use at the next start, is the
//! core's file; the window never sees a sign-in.
//!
//! Which session: Vibe answers `config/proxy/write` only through a session. A write uses a short-lived empty chat
//! (`helper_chat`), so a chat in the middle of a turn can never make Vibe answer "busy" and no session is needed from
//! the user. Reading and testing need no Vibe at all.
//!
//! Depends on: `helper_chat`, `dispatch` (`forward`), `connectors` (`token_for`: the native confirmation),
//! vw-platform (the user-only file), vw-mistral `network`, `network_view`, `network_test`. Called by: `dispatch`.
//! Invariants:
//! - **The window never sees a sign-in.** Addresses leave this module as `scheme://host:port` (or with `***@` in the
//!   field when there is a sign-in), and Vibe's own error texts are scrubbed of the values that were sent.
//! - A write is the user's action: `config/proxy/write` is a confirmed method, so every origin gets the native dialog
//!   (its text shows scheme, host and port only, `vw-gate` `confirm_text`), and nothing is written, here or in the
//!   core's file, without it.
//! - A value is checked before anything is asked (the same rules the gate applies, plus the disk): a refused value
//!   comes back as `refused` with a sentence per field; no helper chat is started for a save that changes nothing.
//! - Saving a proxy always keeps this computer's own addresses on the no-proxy list: Vibe reaches Vibe Work's model
//!   proxy at 127.0.0.1, and with a proxy and an empty list Vibe would send that request to the company proxy.
//! - Saved values apply at the next start (Vibe and the core read them once); `restartNeeded` says so.
//! - CA bytes are fixed before confirmation. Each persisted Save first replaces the vault MAC key, including
//!   removal of all custom settings; interruption before the record replacement rejects the previous record.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use vw_mistral::network::{
    self as net, LOOPBACK, NetworkSettings, PreparedSettings, check_no_proxy, check_path_text,
    check_proxy_url, form_proxy_url, has_sign_in, mask_proxy_url, scrub, with_loopback,
    without_loopback,
};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::network_view::{
    NetworkChanges, NetworkFieldView, NetworkProblem, NetworkRequest, NetworkResult, NetworkView,
};

/// The request fields, the Vibe key each stands for, and the name the user knows it by.
const FIELDS: [(&str, &str, &str); 6] = [
    ("httpProxy", "HTTP_PROXY", "HTTP proxy"),
    ("httpsProxy", "HTTPS_PROXY", "HTTPS proxy"),
    ("allProxy", "ALL_PROXY", "Proxy for all other requests"),
    ("noProxy", "NO_PROXY", "No-proxy list"),
    ("certificateFile", "SSL_CERT_FILE", "Certificate file"),
    ("certificateFolder", "SSL_CERT_DIR", "Certificate folder"),
];

/// What a hidden sign-in looks like in a field (`http://***@host:port`).
const MASK: &str = "***@";

/// The notice for a saved record that is not in use. A record that waits for the vault (no copy of its check key:
/// "Lock the vault when Vibe Work quits" is on, or an install from before the copy) asks for an unlock instead of a
/// new Save; the unlock checks and applies it.
pub(crate) fn rejection_notice(keys: Option<&vw_keys::KeyService>) -> crate::event::CoreEvent {
    let message = match keys.and_then(vw_keys::KeyService::network_settings_wait) {
        Some(vw_keys::NetworkWait::LockAtQuit) => {
            "Saved network settings are used after you unlock the vault (\"Lock the vault when Vibe Work quits\" is on)."
        }
        Some(vw_keys::NetworkWait::NoCopyYet) => {
            "Saved network settings are used after you unlock the vault (it was locked when Vibe Work started)."
        }
        None => {
            "Network settings changed outside Vibe Work were not used. Open Settings > Network and save them again."
        }
    };
    crate::event::CoreEvent::Alert {
        session_id: None,
        kind: crate::event::AlertKind::NetworkSettingsRejected,
        message: message.to_owned(),
    }
}

fn requested<'a>(changes: &'a NetworkChanges, field: &str) -> Option<&'a str> {
    match field {
        "httpProxy" => changes.http_proxy.as_deref(),
        "httpsProxy" => changes.https_proxy.as_deref(),
        "allProxy" => changes.all_proxy.as_deref(),
        "noProxy" => changes.no_proxy.as_deref(),
        "certificateFile" => changes.certificate_file.as_deref(),
        "certificateFolder" => changes.certificate_folder.as_deref(),
        _ => None,
    }
}

/// The values a request means, checked: Vibe key to the new value (`None`: remove). A key whose typed value equals
/// what is saved is not in the map.
#[derive(Debug)]
struct Checked {
    values: BTreeMap<&'static str, Option<String>>,
    settings: NetworkSettings,
}

/// Checks every provided field. Returns the problems when any value fails.
fn check_changes(
    changes: &NetworkChanges,
    saved: &NetworkSettings,
) -> Result<Checked, Vec<NetworkProblem>> {
    let mut values = BTreeMap::new();
    let mut settings = saved.clone();
    let mut problems = Vec::new();
    for (field, key, label) in FIELDS {
        let Some(raw) = requested(changes, field) else {
            continue;
        };
        let current = saved.get(key);
        let typed = raw.trim();
        let outcome: Result<Option<String>, String> = if typed.is_empty() {
            Ok(None)
        } else if matches!(key, "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY") {
            // A field the user did not touch still shows the saved address with its sign-in hidden.
            if current
                .as_deref()
                .is_some_and(|saved| form_proxy_url(saved) == typed)
            {
                continue;
            }
            if typed.contains(MASK) {
                Err(format!(
                    "{label}: The saved sign-in is hidden. Type the whole address again, with its user name and password, or without a sign-in."
                ))
            } else {
                check_proxy_url(label, typed).map(Some)
            }
        } else if key == "NO_PROXY" {
            check_no_proxy(label, typed).map(|list| Some(without_loopback(&list)))
        } else {
            check_path_text(label, typed).map(Some)
        };
        let outcome = outcome.and_then(|value| {
            let value = value.filter(|v| !v.is_empty());
            let mut candidate = settings.clone().with(key, value.clone());
            candidate.snapshot_certificate(key)?;
            settings = candidate;
            Ok(value)
        });
        match outcome {
            Ok(value) => {
                values.insert(key, value);
            }
            Err(message) => problems.push(NetworkProblem {
                field: field.to_owned(),
                message: message
                    .strip_prefix(&format!("{label}: "))
                    .unwrap_or(&message)
                    .to_owned(),
            }),
        }
    }
    if problems.is_empty() {
        Ok(Checked { values, settings })
    } else {
        Err(problems)
    }
}

/// The keys to write to reach `checked` from `saved`, with `NO_PROXY` made right: with a proxy set, the user's list
/// plus this computer's addresses; with none, only what the user typed.
fn writes(saved: &NetworkSettings, checked: &Checked) -> BTreeMap<&'static str, Option<String>> {
    let mut write: BTreeMap<&'static str, Option<String>> = checked
        .values
        .iter()
        .filter(|(key, value)| **key != "NO_PROXY" && saved.get(key) != **value)
        .map(|(key, value)| (*key, value.clone()))
        .collect();
    let after = &checked.settings;
    let user_list = checked.values.get("NO_PROXY").map_or_else(
        || without_loopback(saved.no_proxy.as_deref().unwrap_or_default()),
        |typed| typed.clone().unwrap_or_default(),
    );
    let wanted = if after.has_proxy() {
        Some(with_loopback(&user_list))
    } else if user_list.is_empty() {
        None
    } else {
        Some(user_list)
    };
    if wanted != saved.no_proxy {
        write.insert("NO_PROXY", wanted);
    }
    write
}

/// The labels of the settings the environment overrides.
fn overridden(candidate: &NetworkSettings, environment: &NetworkSettings) -> Vec<String> {
    FIELDS
        .iter()
        .filter(|(_, key, _)| {
            environment.get(key).is_some() && candidate.get(key) != environment.get(key)
        })
        .map(|(_, _, label)| (*label).to_owned())
        .collect()
}

fn field_view(
    key: &str,
    label: &str,
    saved: &NetworkSettings,
    environment: &NetworkSettings,
) -> NetworkFieldView {
    let raw = saved.get(key);
    let is_address = matches!(key, "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY");
    let value = match (&raw, key) {
        (None, _) => String::new(),
        (Some(text), "NO_PROXY") => without_loopback(text),
        (Some(text), _) if is_address => form_proxy_url(text),
        (Some(text), _) => text.clone(),
    };
    let problem = raw.as_deref().and_then(|text| match key {
        _ if is_address => check_proxy_url(label, text).err(),
        _ => None,
    });
    NetworkFieldView {
        value,
        has_sign_in: is_address && raw.as_deref().is_some_and(has_sign_in),
        environment_value: environment.get(key).map(|text| {
            if is_address {
                mask_proxy_url(&text)
            } else {
                text
            }
        }),
        problem: problem.map(|p| {
            p.strip_prefix(&format!("{label}: "))
                .map_or(p.clone(), str::to_owned)
        }),
    }
}

/// The settings as the screen shows them.
fn view_of(
    saved: &NetworkSettings,
    environment: &NetworkSettings,
    in_use: &NetworkSettings,
) -> NetworkView {
    let field = |key: &str| {
        let label = FIELDS
            .iter()
            .find(|(_, k, _)| *k == key)
            .map_or(key, |(_, _, label)| *label);
        field_view(key, label, saved, environment)
    };
    let certificate_count = saved
        .saved_certificate_count()
        .and_then(|count| u32::try_from(count).ok());
    NetworkView {
        http_proxy: field("HTTP_PROXY"),
        https_proxy: field("HTTPS_PROXY"),
        all_proxy: field("ALL_PROXY"),
        no_proxy: field("NO_PROXY"),
        certificate_file: field("SSL_CERT_FILE"),
        certificate_folder: field("SSL_CERT_DIR"),
        always_direct: LOOPBACK.iter().map(|s| (*s).to_owned()).collect(),
        restart_needed: NetworkSettings::effective(saved, environment) != *in_use,
        certificate_count,
    }
}

/// `saved` with `write` laid over it (the settings after a successful write).
fn after_write(
    saved: &NetworkSettings,
    write: &BTreeMap<&'static str, Option<String>>,
) -> NetworkSettings {
    write.iter().fold(saved.clone(), |settings, (key, value)| {
        settings.with(key, value.clone())
    })
}

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
        // A disk-confirmed write (the file is synced): on the blocking pool, never on an async worker.
        let file = file.to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            file.parent()
                .map_or(Ok(()), vw_platform::ensure_private_dir)
                .and_then(|()| vw_platform::replace_private(&file, text.as_bytes()))
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

    async fn network_read(&self) -> NetworkResult {
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
        let probe = net::probe(&used, &self.shared.config.network_test_url).await;
        let mut outcome =
            crate::network_test::describe(probe, &used, overridden(tried, &environment));
        crate::network_python::verify(&self.shared, &used, &mut outcome).await;
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
        let params = json!({"sessionId": session, "changes": changes});
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
        if !fingerprints.is_empty() {
            self.shared
                .dialog_texts
                .set_suffix(method, &params, fingerprints);
        }
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
        self.forward(origin, method, params, token.map(|t| t.as_str().to_owned()))
            .await
            .map_err(scrubbed)?;
        let after = self.read_vibe(origin, session).await?;
        for (key, wanted) in write {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> NetworkSettings {
        pairs.iter().fold(NetworkSettings::default(), |s, (k, v)| {
            s.with(k, Some((*v).to_owned()))
        })
    }

    fn changes(f: impl FnOnce(&mut NetworkChanges)) -> NetworkChanges {
        let mut c = NetworkChanges::default();
        f(&mut c);
        c
    }

    #[test]
    fn an_untouched_masked_field_is_not_a_change_and_a_half_masked_one_is_refused() {
        let saved = settings(&[("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:8080")]);
        let untouched = changes(|c| c.https_proxy = Some("http://***@proxy.corp:8080".to_owned()));
        let checked = check_changes(&untouched, &saved).expect("fine");
        assert!(checked.values.is_empty());

        let half = changes(|c| c.https_proxy = Some("http://***@other.corp:8080".to_owned()));
        let problems = check_changes(&half, &saved).expect_err("refused");
        assert_eq!(problems[0].field, "httpsProxy");
        assert!(problems[0].message.contains("hidden"), "{problems:?}");
        assert!(!problems[0].message.contains("s3cret"));

        // The same host typed without the marker is a change: the sign-in is removed.
        let plain = changes(|c| c.https_proxy = Some("http://proxy.corp:8080".to_owned()));
        let checked = check_changes(&plain, &saved).expect("fine");
        assert_eq!(
            checked.values.get("HTTPS_PROXY"),
            Some(&Some("http://proxy.corp:8080".to_owned()))
        );
    }

    #[test]
    fn a_blank_field_removes_and_a_wrong_value_names_its_field_without_quoting_it() {
        let saved = settings(&[("HTTP_PROXY", "http://p:1")]);
        let input = changes(|c| {
            c.http_proxy = Some("  ".to_owned());
            c.https_proxy = Some("secret-user:pw@host:1".to_owned());
            c.certificate_file = Some("/definitely/not/here.pem".to_owned());
        });
        let problems = check_changes(&input, &saved).expect_err("two problems");
        let fields: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields, ["httpsProxy", "certificateFile"]);
        assert!(problems.iter().all(|p| !p.message.contains("secret-user")));
        let only_blank = changes(|c| c.http_proxy = Some(String::new()));
        let checked = check_changes(&only_blank, &saved).expect("fine");
        assert_eq!(checked.values.get("HTTP_PROXY"), Some(&None));
    }

    #[test]
    fn saving_a_proxy_always_puts_this_computers_addresses_on_the_no_proxy_list() {
        let none = NetworkSettings::default();
        let input = changes(|c| c.https_proxy = Some("http://proxy.corp:8080".to_owned()));
        let checked = check_changes(&input, &none).expect("fine");
        let write = writes(&none, &checked);
        assert_eq!(
            write["HTTPS_PROXY"].as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(
            write["NO_PROXY"].as_deref(),
            Some("localhost,127.0.0.1,::1")
        );

        // The user's list is kept, loopback is not duplicated, and typing it again changes nothing.
        let saved = settings(&[
            ("HTTPS_PROXY", "http://proxy.corp:8080"),
            ("NO_PROXY", ".corp,localhost,127.0.0.1,::1"),
        ]);
        let again = changes(|c| c.no_proxy = Some(".corp".to_owned()));
        let checked = check_changes(&again, &saved).expect("fine");
        assert!(writes(&saved, &checked).is_empty());
        let more = changes(|c| c.no_proxy = Some(".corp, 10.0.0.0/8".to_owned()));
        let checked = check_changes(&more, &saved).expect("fine");
        assert_eq!(
            writes(&saved, &checked)["NO_PROXY"].as_deref(),
            Some(".corp,10.0.0.0/8,localhost,127.0.0.1,::1")
        );
    }

    #[test]
    fn removing_the_last_proxy_removes_the_list_this_module_added() {
        let saved = settings(&[
            ("HTTPS_PROXY", "http://proxy.corp:8080"),
            ("NO_PROXY", "localhost,127.0.0.1,::1"),
        ]);
        let input = changes(|c| c.https_proxy = Some(String::new()));
        let checked = check_changes(&input, &saved).expect("fine");
        let write = writes(&saved, &checked);
        assert_eq!(write["HTTPS_PROXY"], None);
        assert_eq!(write["NO_PROXY"], None);
        // A list of the user's own stays.
        let own = settings(&[
            ("HTTPS_PROXY", "http://p:1"),
            ("NO_PROXY", ".corp,localhost,127.0.0.1,::1"),
        ]);
        let checked = check_changes(&input, &own).expect("fine");
        assert_eq!(writes(&own, &checked)["NO_PROXY"].as_deref(), Some(".corp"));
    }

    #[test]
    fn the_view_hides_the_sign_in_and_names_what_the_environment_overrides() {
        let saved = settings(&[
            ("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:8080"),
            ("NO_PROXY", ".corp,localhost,127.0.0.1,::1"),
        ]);
        let environment = settings(&[("HTTPS_PROXY", "http://env:pw@envproxy:3128")]);
        let view = view_of(&saved, &environment, &NetworkSettings::default());
        let shown = format!("{view:?}");
        assert!(
            !shown.contains("s3cret") && !shown.contains("alice") && !shown.contains("pw@"),
            "{shown}"
        );
        assert_eq!(view.https_proxy.value, "http://***@proxy.corp:8080");
        assert!(view.https_proxy.has_sign_in);
        assert_eq!(
            view.https_proxy.environment_value.as_deref(),
            Some("http://envproxy:3128")
        );
        assert_eq!(view.no_proxy.value, ".corp");
        assert_eq!(view.always_direct, ["localhost", "127.0.0.1", "::1"]);
        let candidate = NetworkSettings::default();
        assert_eq!(
            overridden(
                &candidate.with("HTTPS_PROXY", Some("http://x:1".into())),
                &environment
            ),
            ["HTTPS proxy"]
        );
    }

    #[test]
    fn a_saved_change_waits_for_a_restart_until_this_run_uses_it() {
        let saved = settings(&[
            ("HTTPS_PROXY", "http://p:1"),
            ("NO_PROXY", "localhost,127.0.0.1,::1"),
        ]);
        let environment = NetworkSettings::default();
        let before = NetworkSettings::default();
        assert!(view_of(&saved, &environment, &before).restart_needed);
        let after = NetworkSettings::effective(&saved, &environment);
        assert!(!view_of(&saved, &environment, &after).restart_needed);
    }

    #[test]
    fn every_field_maps_to_one_of_vibes_keys() {
        for (_, key, _) in FIELDS {
            assert!(net::KEYS.contains(&key));
        }
        assert_eq!(FIELDS.len(), net::KEYS.len());
    }
}
