//! Tools skills operations through vw-skills and the selected session's actual runtime.
//! Without a selected chat, or when the selected chat has no live channel, uses a disposable helper in the picked
//! folder or the chat's own folder; gate forwarding owns transport.
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::skills_rows::{representative, row_view};
use crate::skills_view::{SkillDetail, SkillsAction, SkillsRequest, SkillsResult};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vw_platform::sync::LockRecover;
use vw_skills::{SkillsClient, SkillsTransport, TransportError};

#[path = "skills_change.rs"]
mod change;
use change::mutation_target;
#[path = "skills_offer.rs"]
mod offer;

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
                *self.refusal.lock_or_recover() = Some(error.clone());
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
        .lock_or_recover()
        .take()
        .unwrap_or_else(|| failure(e))
}
fn refuse(text: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, text)
}
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
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
        if let Some(id) = request
            .session_id
            .as_deref()
            .filter(|id| self.shared.channels.has_channel(id))
        {
            // Keep session-only trust, extra roots, agent overrides and pinned plugins exactly
            // as the selected runtime sees them. Never dispose of the person's own chat.
            return self.skills_in(origin, id, &folder, request.action).await;
        }
        // No selected chat, or one without a live runtime (closed after being idle, or from an earlier run):
        // Vibe answers runtime reads for a closed chat with empty lists, and reopening it could restart its
        // unfinished work. A short-lived helper in the chat's folder reads (and changes) what that folder's chats
        // load; the closed chat picks the change up when it is next opened.
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
}
