//! Tools skills operations through vw-skills and the selected session's actual runtime.
//! Without a selected chat, uses a disposable helper in the picked folder; gate forwarding owns transport.
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::keys::mistral_key_available;
use crate::skills_rows::{board, lock_reason, representative, row_view};
use crate::skills_view::{SkillDetail, SkillsAction, SkillsRequest, SkillsResult};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vw_skills::{SkillsClient, SkillsTransport, TransportError};

pub(crate) const BUSY: &str = "Wait until the chats in this folder finish, then change skills.";

struct Transport<'a> {
    core: &'a Core,
    origin: Origin,
    /// The window's click confirmed a shown install offer: install and update need no native dialog.
    offered: bool,
    /// The confirmation step's own error (for example Cancel in the native dialog), kept whole so
    /// the caller keeps its code: the skills client passes only a message through.
    refusal: &'a Mutex<Option<CoreError>>,
}
impl SkillsTransport for Transport<'_> {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        let result = async {
            // Install and update give an agent new instructions or code: from the window they need the click on
            // an offer it showed first (`install_offers`), otherwise the native dialog; the Debug API is
            // answered by its confirmer. Reads and other calls need no token.
            let token = if self.offered && vw_gate::OFFERED_INSTALL_METHODS.contains(&method) {
                self.core
                    .shared
                    .gate
                    .request_offered_confirmation(method, &params)
                    .map(Some)
                    .map_err(|e| CoreError::from_gate(method, &e))
            } else {
                self.core.token_for(self.origin, method, &params).await
            }
            .inspect_err(|error| {
                *self
                    .refusal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.clone());
            })?;
            self.core
                .forward(
                    self.origin,
                    method,
                    params,
                    token.map(|t| t.as_str().to_owned()),
                )
                .await
        }
        .await;
        result
            .map(|r| r.result)
            .map_err(|e| TransportError::Unavailable(e.message))
    }
}
fn failure(e: &vw_skills::SkillsError) -> CoreError {
    CoreError::new(ErrorCode::ServerError, e.to_string())
}
/// A failed change: the confirmation step's own error when that step refused (its code tells the
/// window that the user cancelled), otherwise Vibe's reason as a server error.
fn change_failure(refusal: &Mutex<Option<CoreError>>, e: &vw_skills::SkillsError) -> CoreError {
    refusal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .unwrap_or_else(|| failure(e))
}
fn refuse(text: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, text)
}
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
/// The version an install offer names for `skill_id` of `row`: the catalog's latest, else the row's (one place, so
/// the offer and the check at the click read it the same way).
fn offered_version(
    row: &vw_skills::SkillView,
    skill_id: &str,
    row_latest: Option<i64>,
) -> Option<i64> {
    row.catalog
        .iter()
        .find(|c| c.skill_id == skill_id)
        .map(|c| c.latest_version)
        .or(row_latest)
}

/// The registry change of an installed skill: remove it unless `updating`. An offered update (`pinned`) installs
/// and pins that version (`skills/setVersion`), because Vibe's `setLatest` fetches whatever `latest` names at that
/// moment and keeps following it; an update the native dialog confirmed follows `latest`.
async fn change_installed<T: SkillsTransport>(
    client: &SkillsClient<T>,
    id: &str,
    name: String,
    scope: String,
    updating: bool,
    pinned: Option<i64>,
) -> Result<(), vw_skills::SkillsError> {
    let session_id = id.to_owned();
    match (updating, pinned) {
        (true, Some(version)) => client
            .set_version(&vw_skills::SkillsSetVersionParams {
                session_id,
                name,
                version,
                scope,
            })
            .await
            .map(drop),
        (true, None) => client
            .set_latest(&vw_skills::SkillsSetLatestParams {
                session_id,
                name,
                scope,
            })
            .await
            .map(drop),
        (false, _) => client
            .remove(&vw_skills::SkillsRemoveParams {
                session_id,
                name,
                scope,
            })
            .await
            .map(drop),
    }
}

/// The `serverCode` of an offered update that ran while Vibe now reports another version (or none) installed.
pub(crate) const UPDATED_OTHER_VERSION: &str = "updatedOtherVersion";

/// An offered update counts as done only when Vibe now reports the offered version installed. Otherwise the change
/// did run (another Vibe sharing the skills folder can change it in between): the answer says so and what Vibe
/// reports now, and nothing is undone (a rollback could overwrite that other change). The window then reads the
/// list again.
fn offered_update_landed(
    view: &vw_skills::SkillsView,
    name: Option<&str>,
    version: i64,
) -> Result<(), CoreError> {
    let installed = view
        .skills
        .iter()
        .find(|r| Some(r.name.as_str()) == name)
        .and_then(|r| r.installed_version);
    if installed == Some(version) {
        return Ok(());
    }
    let now = installed.map_or_else(
        || "Vibe no longer reports this skill installed".to_owned(),
        |other| format!("Vibe now reports version {other} installed"),
    );
    let mut error = CoreError::new(
        ErrorCode::ServerError,
        format!(
            "The update to version {version} ran, but {now}. Nothing was undone; the list now shows what is installed."
        ),
    );
    error.server_code = Some(UPDATED_OTHER_VERSION.to_owned());
    Err(error)
}

fn mutation_target<'a>(
    view: &'a vw_skills::SkillsView,
    action: &SkillsAction,
) -> Result<Option<&'a vw_skills::SkillView>, CoreError> {
    let name = match action {
        SkillsAction::SetEnabled { name, .. }
        | SkillsAction::SetLatest { name, .. }
        | SkillsAction::OfferUpdate { name }
        | SkillsAction::Remove { name } => Some(name.as_str()),
        SkillsAction::Import { skill_id, .. } | SkillsAction::OfferImport { skill_id } => view
            .skills
            .iter()
            .find(|r| r.catalog.iter().any(|c| &c.skill_id == skill_id))
            .map(|r| r.name.as_str()),
        _ => None,
    };
    let target = name.and_then(|n| view.skills.iter().find(|r| r.name == n));
    if !matches!(action, SkillsAction::Read {}) {
        let row = target
            .ok_or_else(|| refuse("The skill is no longer available. Read the list again."))?;
        if let Some(reason) = lock_reason(row) {
            return Err(refuse(reason));
        }
        if !row.installations.is_empty() && representative(row).is_none() {
            return Err(refuse(
                "This skill has multiple installed copies. Choose it in Vibe's settings file.",
            ));
        }
    }
    Ok(target)
}
impl Core {
    pub(crate) async fn lock_skills_work(
        &self,
        method: &str,
    ) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        if crate::changes::starts_work(method) || method == "session/turn/queue/resume" {
            Some(self.shared.skills_lock.lock().await)
        } else {
            None
        }
    }
    fn skills_folder(&self, request: &SkillsRequest) -> Result<PathBuf, CoreError> {
        if let Some(id) = request.session_id.as_deref() {
            return self
                .shared
                .store
                .view(id)
                .and_then(|v| {
                    v.state
                        .pointer("/session/cwd")
                        .and_then(Value::as_str)
                        .map(PathBuf::from)
                })
                .ok_or_else(|| refuse("The selected chat's work folder is unavailable."));
        }
        if let Some(cwd) = &request.cwd {
            let path = PathBuf::from(cwd);
            if !self.shared.gate.is_picked_folder(&path) {
                return Err(refuse(
                    "Select the work folder with the folder picker first.",
                ));
            }
            return Ok(path);
        }
        Ok(self.shared.supervisor.work_dir())
    }
    fn skills_busy(&self, folder: &Path) -> bool {
        let folder = canonical(folder);
        self.shared.store.views().iter().any(|v| {
            v.state
                .pointer("/session/cwd")
                .and_then(Value::as_str)
                .is_some_and(|cwd| canonical(Path::new(cwd)) == folder)
                && matches!(
                    v.state
                        .pointer("/session/status/type")
                        .and_then(Value::as_str),
                    Some("running" | "blocked")
                )
        })
    }
    // The turn reply may beat its status notification. With work admission locked, read the server's
    // current status as well before relying on an idle cached view.
    async fn skills_check_idle(&self, folder: &Path) -> Result<(), CoreError> {
        let folder = canonical(folder);
        let ids: Vec<_> = self
            .shared
            .store
            .views()
            .into_iter()
            .filter(|v| {
                v.state
                    .pointer("/session/cwd")
                    .and_then(Value::as_str)
                    .is_some_and(|cwd| canonical(Path::new(cwd)) == folder)
            })
            .map(|v| v.session_id)
            .collect();
        for id in ids {
            let reply = self
                .shared
                .gate
                .call(
                    "session/read",
                    json!({"sessionId":id,"history":{"limit":1},"turns":{"limit":1}}),
                    None,
                )
                .await
                .map_err(|e| CoreError::from_gate("session/read", &e))?;
            if matches!(
                reply
                    .result
                    .pointer("/state/session/status/type")
                    .and_then(Value::as_str),
                Some("running" | "blocked")
            ) {
                return Err(refuse(BUSY));
            }
        }
        Ok(())
    }
    pub(crate) async fn skills(
        &self,
        origin: Origin,
        request: SkillsRequest,
    ) -> Result<SkillsResult, CoreError> {
        let folder = self.skills_folder(&request)?;
        let mutation = !matches!(
            request.action,
            SkillsAction::Read {} | SkillsAction::Detail { .. }
        );
        let _guard = if mutation {
            Some(self.shared.skills_lock.lock().await)
        } else {
            None
        };
        if mutation && self.skills_busy(&folder) {
            return Err(refuse(BUSY));
        }
        if mutation {
            self.skills_check_idle(&folder).await?;
        }
        if let Some(id) = request.session_id.as_deref() {
            // Keep session-only trust, extra roots, agent overrides and pinned plugins exactly
            // as the selected runtime sees them. Never dispose of the person's own chat.
            return self.skills_in(origin, id, &folder, request.action).await;
        }
        let id = self
            .start_helper_chat_in(origin, "Skills were not changed", &folder)
            .await?;
        let result = self.skills_in(origin, &id, &folder, request.action).await;
        if !self.end_helper_chat(&id).await {
            tracing::warn!(
                session_id = id,
                "skills helper remains temporary until shutdown"
            );
        }
        result
    }
    async fn skills_read(
        &self,
        origin: Origin,
        id: &str,
    ) -> Result<vw_skills::SkillsView, CoreError> {
        let refusal = Mutex::new(None);
        let client = SkillsClient::new(Transport {
            core: self,
            origin,
            offered: false,
            refusal: &refusal,
        });
        let installed = client
            .installed(&vw_skills::SkillsInstalledParams {
                session_id: id.to_owned(),
            })
            .await
            .map_err(|e| failure(&e))?;
        let active = client
            .list(&vw_skills::SkillsListParams {
                session_id: Some(id.to_owned()),
                cwd: None,
                trust_workspace: false,
            })
            .await
            .map_err(|e| failure(&e))?;
        let catalog = client
            .catalog(&vw_skills::SkillsCatalogParams {
                session_id: id.to_owned(),
            })
            .await
            .unwrap_or(vw_skills::SkillsCatalogResponse {
                skills: vec![],
                updates: std::collections::BTreeMap::default(),
                loaded: false,
                authenticated: true,
                project_available: false,
            });
        let runtime = self
            .forward(origin, "runtime/read", json!({"sessionId":id}), None)
            .await?;
        let issues: Vec<vw_skills::ConfigIssue> = serde_json::from_value(
            runtime
                .result
                .pointer("/runtime/issues")
                .cloned()
                .unwrap_or(json!([])),
        )
        .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(vw_skills::SkillsView::merge(
            &installed,
            &catalog,
            &vw_skills::SkillsUpdatesResponse { updates: vec![] },
            &active,
            &issues,
        ))
    }
    async fn skills_in(
        &self,
        origin: Origin,
        id: &str,
        folder: &Path,
        action: SkillsAction,
    ) -> Result<SkillsResult, CoreError> {
        let before = self.skills_read(origin, id).await?;
        let refusal = Mutex::new(None);
        // an install or update naming a shown offer takes it first (single use, also when the change
        // fails); only the window's click then skips the native dialog.
        let (offered, pinned) = self.offered_skill_change(origin, &action, folder, &before)?;
        let client = SkillsClient::new(Transport {
            core: self,
            origin,
            offered,
            refusal: &refusal,
        });
        if let Some(answer) = self
            .skills_view_only(&client, id, folder, &before, &action)
            .await
        {
            return answer;
        }
        if !matches!(action, SkillsAction::Read {}) && self.skills_busy(folder) {
            return Err(refuse(BUSY));
        }
        let target = mutation_target(&before, &action)?;
        let row = target.map(row_view);
        let updating = matches!(action, SkillsAction::SetLatest { .. });
        let target_name = row.as_ref().map(|r| r.name.clone());
        let changed = match action {
            SkillsAction::Read {} => false,
            SkillsAction::SetEnabled { name, enabled } => {
                if !row.as_ref().is_some_and(|r| r.installed) {
                    return Err(refuse("Install this skill first."));
                }
                client
                    .set_enabled(&vw_skills::SkillsSetEnabledParams {
                        session_id: id.to_owned(),
                        name,
                        enabled,
                    })
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::Import { skill_id, .. } => {
                if row.as_ref().is_some_and(|r| r.installed) {
                    return Err(refuse("This skill is already installed."));
                }
                client
                    .import(&vw_skills::SkillsImportParams {
                        session_id: id.to_owned(),
                        skill_id,
                        // The version a shown offer named (the click installs what the user saw); latest
                        // otherwise (the native dialog path).
                        version: pinned,
                        alias: None,
                        scope: "global".to_owned(),
                    })
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::SetLatest { name, .. } | SkillsAction::Remove { name } => {
                let row = row.ok_or_else(|| refuse("The skill is unavailable."))?;
                if !row.installed || row.skill_id.is_none() {
                    return Err(refuse(
                        "Only registry-installed skills can be updated or removed.",
                    ));
                }
                let scope = row
                    .scope
                    .ok_or_else(|| refuse("The skill's install scope is ambiguous."))?;
                change_installed(&client, id, name, scope, updating, pinned)
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::Detail { .. }
            | SkillsAction::OfferImport { .. }
            | SkillsAction::OfferUpdate { .. } => unreachable!(),
        };
        let view = if changed {
            self.skills_read(origin, id).await?
        } else {
            before
        };
        if let (true, Some(version)) = (updating, pinned) {
            offered_update_landed(&view, target_name.as_deref(), version)?;
        }
        let authenticated = mistral_key_available(self.shared.config.keys.as_deref());
        let busy = self.skills_busy(folder);
        let places =
            crate::skill_times::SkillPlaces::new(&self.shared.instructions.own_home, folder);
        // The rows' file times read the disk: off the async workers.
        let view = tokio::task::spawn_blocking(move || board(view, busy, authenticated, &places))
            .await
            .map_err(|e| CoreError::new(ErrorCode::Internal, format!("skills list failed: {e}")))?;
        Ok(SkillsResult::View { view })
    }

    async fn skills_detail<T: SkillsTransport>(
        &self,
        client: &SkillsClient<T>,
        id: &str,
        folder: &Path,
        view: &vw_skills::SkillsView,
        name: &str,
    ) -> Result<SkillDetail, CoreError> {
        let row = view
            .skills
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| refuse("The skill is no longer available."))?;
        let shown = row_view(row);
        let mut detail = SkillDetail {
            name: name.to_owned(),
            description: shown.description,
            body: representative(row)
                .map(|s| s.prompt.clone())
                .filter(|s| !s.is_empty()),
            versions: vec![],
            locations: row
                .installations
                .iter()
                .map(|s| {
                    format!(
                        "{:?} · {:?}{}",
                        s.source,
                        s.scope,
                        if s.scope == vw_skills::SkillScope::Project {
                            format!(" · {}", folder.display())
                        } else {
                            String::new()
                        }
                    )
                })
                .collect(),
        };
        if let Some(skill_id) = shown.skill_id {
            let response = client
                .detail(&vw_skills::SkillsDetailParams {
                    session_id: id.to_owned(),
                    skill_id: skill_id.clone(),
                    version: shown.installed_version,
                })
                .await
                .map_err(|e| failure(&e))?;
            if let Some(registry) = response.detail {
                detail.description = registry.description;
                detail.body = Some(registry.body);
            } else if response.body.is_some() {
                detail.body = response.body;
            }
            detail.versions = client
                .versions(&vw_skills::SkillsVersionsParams {
                    session_id: id.to_owned(),
                    skill_id,
                })
                .await
                .map_err(|e| failure(&e))?
                .versions
                .iter()
                .map(|v| format!("v{}", v.version))
                .collect();
        }
        Ok(detail)
    }

    /// The operations that change nothing: `detail` and the install offers. `None` for every other operation.
    async fn skills_view_only<T: SkillsTransport>(
        &self,
        client: &SkillsClient<T>,
        id: &str,
        folder: &Path,
        before: &vw_skills::SkillsView,
        action: &SkillsAction,
    ) -> Option<Result<SkillsResult, CoreError>> {
        match action {
            SkillsAction::Detail { name } => Some(
                self.skills_detail(client, id, folder, before, name)
                    .await
                    .map(|detail| SkillsResult::Detail { detail }),
            ),
            SkillsAction::OfferImport { .. } | SkillsAction::OfferUpdate { .. } => {
                let target = match mutation_target(before, action) {
                    Ok(target) => target,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    self.skill_offer(client, id, folder, target, action)
                        .await
                        .map(|offer| SkillsResult::Offer { offer }),
                )
            }
            _ => None,
        }
    }

    /// [`Self::take_skill_offer`] for an `import` or `setLatest` that names an offer: whether the click may skip the
    /// native dialog, and the version the change installs (the offered one). `(false, None)` for anything else.
    ///
    /// The offer is bound to what it showed: an import pins the offered version; an update is refused as a stale
    /// offer when the registry's latest is no longer the offered one in the view read right before (`before`), and
    /// otherwise installs the offered number with `skills/setVersion` (never the moving `latest`).
    fn offered_skill_change(
        &self,
        origin: Origin,
        action: &SkillsAction,
        folder: &Path,
        before: &vw_skills::SkillsView,
    ) -> Result<(bool, Option<i64>), CoreError> {
        match action {
            SkillsAction::Import {
                skill_id,
                offer_id: Some(offer),
            } => self.take_skill_offer(origin, offer, "skills/import", skill_id, folder),
            SkillsAction::SetLatest {
                name,
                offer_id: Some(offer),
            } => {
                let (skip, shown) =
                    self.take_skill_offer(origin, offer, "skills/setLatest", name, folder)?;
                let now = mutation_target(before, action)?.and_then(|row| {
                    let view = row_view(row);
                    view.skill_id
                        .as_deref()
                        .and_then(|skill_id| offered_version(row, skill_id, view.latest_version))
                });
                if now != shown {
                    return Err(crate::install_offers::changed());
                }
                // The update pins the offered number (`skills/setVersion`); an offer without one is not installed.
                let version = shown.ok_or_else(crate::install_offers::changed)?;
                Ok((skip, Some(version)))
            }
            _ => Ok((false, None)),
        }
    }

    /// Takes the window's install offer for this exact action; true when the click may skip the native dialog
    /// (the window's origin only; the Debug API keeps its own confirmer path).
    fn take_skill_offer(
        &self,
        origin: Origin,
        offer: &str,
        method: &str,
        target: &str,
        folder: &Path,
    ) -> Result<(bool, Option<i64>), CoreError> {
        let key = crate::install_offers::OfferKey::new(
            method,
            target,
            &canonical(folder).to_string_lossy(),
        );
        let version = self
            .shared
            .install_offers
            .take(offer, &key, self.shared.asks.now_ms())?;
        Ok((origin == Origin::Ui, version))
    }

    /// The offer the window shows before Install or Update: Vibe's catalog name and version, the registry as the
    /// source, the author Vibe reports, and what a skill adds. Vibe's registry states no licence.
    async fn skill_offer<T: SkillsTransport>(
        &self,
        client: &SkillsClient<T>,
        id: &str,
        folder: &Path,
        target: Option<&vw_skills::SkillView>,
        action: &SkillsAction,
    ) -> Result<crate::install_offers::ToolsInstallOffer, CoreError> {
        let row = target.ok_or_else(|| refuse("The skill is unavailable."))?;
        let shown = row_view(row);
        let (kind, method, key_target, skill_id) =
            if let SkillsAction::OfferImport { skill_id } = action {
                if shown.installed {
                    return Err(refuse("This skill is already installed."));
                }
                ("skill", "skills/import", skill_id.clone(), skill_id.clone())
            } else {
                if !shown.installed || shown.skill_id.is_none() || !shown.update_available {
                    return Err(refuse("There is no update for this skill."));
                }
                (
                    "skillUpdate",
                    "skills/setLatest",
                    row.name.clone(),
                    shown.skill_id.clone().unwrap_or_default(),
                )
            };
        let version = offered_version(row, &skill_id, shown.latest_version);
        let publisher = client
            .detail(&vw_skills::SkillsDetailParams {
                session_id: id.to_owned(),
                skill_id: skill_id.clone(),
                version,
            })
            .await
            .ok()
            .and_then(|d| d.detail)
            .map(|d| d.created_by)
            .filter(|by| !by.trim().is_empty());
        let offer = crate::install_offers::ToolsInstallOffer {
            offer_id: String::new(),
            kind: kind.to_owned(),
            name: row.name.clone(),
            publisher,
            source: format!("Mistral skill registry ({skill_id})"),
            licence: None,
            runs: "Instructions the agent may follow in every project. Installing runs no program."
                .to_owned(),
            version,
            expires_at: 0,
        };
        let key = crate::install_offers::OfferKey::new(
            method,
            &key_target,
            &canonical(folder).to_string_lossy(),
        );
        self.shared
            .install_offers
            .make(key, offer, self.shared.asks.now_ms())
    }
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
