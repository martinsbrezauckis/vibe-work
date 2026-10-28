//! Finishes authorized Vibe deletes after storage errors; keeps personal retries separate from temporary chats.
//! Uses the gate's confirmed result, pinned launch storage and `session_delete_files`' handle walker.
//! Called by `worktrees_deleted` and core startup; local/gate refusals never grant filesystem deletion authority.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use vw_gate::{GateError, Reply};
use vw_rpc::RpcError;
use vw_supervisor::SupervisorConfig;

use crate::core::{Core, Shared};
use crate::session_delete_files::{self as files, Targets};

const MESSAGE: &str = "Vibe Work could not remove every file of this chat yet. It tries again the next time it starts.";
const RECORD: &str = "pending-deletes.json";

pub(crate) struct Deletes {
    pending: Mutex<BTreeMap<String, Targets>>,
    file: Option<PathBuf>,
    frozen: bool,
    locks: Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>,
    /// Check the unexpanded configured spelling too: Vibe resolves links while loading settings.
    configured_dir: PathBuf,
    prefix: String,
}

fn absolute_directory(server: &SupervisorConfig, configured_dir: &std::path::Path) -> PathBuf {
    if let Ok(rest) = configured_dir.strip_prefix("~") {
        let home = server.home.clone().or_else(|| {
            let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            server
                .extra_env
                .iter()
                .rev()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| PathBuf::from(value))
                .or_else(|| {
                    server.parent_env.as_ref().and_then(|env| {
                        env.iter()
                            .rev()
                            .find(|(name, _)| name.eq_ignore_ascii_case(key))
                            .map(|(_, value)| PathBuf::from(value))
                    })
                })
                .or_else(|| std::env::var_os(key).map(PathBuf::from))
        });
        home.map_or_else(|| configured_dir.to_path_buf(), |home| home.join(rest))
    } else if configured_dir.to_string_lossy().starts_with('~') {
        // Preserve Python's named-user expansion; an unresolved spelling cannot grant host cleanup.
        configured_dir.to_path_buf()
    } else if configured_dir.is_relative() {
        server.cwd.join(configured_dir)
    } else {
        configured_dir.to_path_buf()
    }
}

impl Deletes {
    pub(crate) fn configure(server: &mut SupervisorConfig, data: Option<&std::path::Path>) -> Self {
        let file = data.map(|dir| dir.join(RECORD));
        let mut frozen = false;
        let pending = file
            .as_ref()
            .and_then(|file| match std::fs::read(file) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(ids) if bytes.len() <= 1024 * 1024 => Some(ids),
                    _ => {
                        frozen = true;
                        tracing::error!("pending chat deletes: damaged record left untouched");
                        None
                    }
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => {
                    frozen = true;
                    tracing::error!(%e, "pending chat deletes: unreadable record left untouched");
                    None
                }
            })
            .unwrap_or_default();
        let configured_dir = server
            .extra_env
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case("VIBE_SESSION_LOGGING__SAVE_DIR"))
            .map(|(_, path)| {
                if path.is_empty() {
                    server.vibe_home.join("logs/session")
                } else {
                    PathBuf::from(path)
                }
            })
            .or_else(|| {
                let text = std::fs::read_to_string(server.vibe_home.join("config.toml")).ok()?;
                let value: toml::Value = toml::from_str(&text).ok()?;
                value
                    .get("session_logging")?
                    .get("save_dir")?
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| server.vibe_home.join("logs/session"));
        let configured_dir = absolute_directory(server, &configured_dir);
        let prefix = server
            .extra_env
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case("VIBE_SESSION_LOGGING__SESSION_PREFIX"))
            .and_then(|(_, value)| value.to_str().map(str::to_owned))
            .or_else(|| {
                let text = std::fs::read_to_string(server.vibe_home.join("config.toml")).ok()?;
                let value: toml::Value = toml::from_str(&text).ok()?;
                value
                    .get("session_logging")?
                    .get("session_prefix")?
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "session".into());
        // One launch has one storage root. Pin the selected home/CLI configuration in the exact
        // environment Vibe receives; recovery then needs neither a model runtime nor a key/network.
        server.extra_env.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("VIBE_SESSION_LOGGING__SAVE_DIR")
                && !name.eq_ignore_ascii_case("VIBE_SESSION_LOGGING__SESSION_PREFIX")
        });
        server.extra_env.extend([
            (
                "VIBE_SESSION_LOGGING__SAVE_DIR".into(),
                configured_dir.clone().into_os_string(),
            ),
            (
                "VIBE_SESSION_LOGGING__SESSION_PREFIX".into(),
                prefix.clone().into(),
            ),
        ]);
        Self {
            prefix,
            pending: Mutex::new(pending),
            file,
            frozen,
            locks: Mutex::default(),
            configured_dir,
        }
    }

    pub(crate) fn ids(&self) -> Vec<String> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    fn targets(&self, id: &str) -> Option<Targets> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    pub(crate) async fn lock(&self, id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    async fn remember(self: &Arc<Self>, id: &str, targets: Targets) -> io::Result<()> {
        let (this, id) = (self.clone(), id.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut pending = this.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending.insert(id, targets);
            this.write(&pending)
        })
        .await
        .map_err(io::Error::other)?
    }

    fn write(&self, pending: &BTreeMap<String, Targets>) -> io::Result<()> {
        if self.frozen {
            return Err(io::Error::other("pending delete record is damaged"));
        }
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| io::Error::other("no durable delete record configured"))?;
        if let Some(parent) = file.parent() {
            vw_platform::ensure_private_dir(parent).map_err(io::Error::other)?;
        }
        vw_platform::replace_private(file, &serde_json::to_vec(pending)?).map_err(io::Error::other)
    }

    pub(crate) async fn forget(self: &Arc<Self>, id: &str) {
        let (this, id) = (self.clone(), id.to_owned());
        let _ = tokio::task::spawn_blocking(move || {
            let mut pending = this.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.remove(&id).is_some()
                && let Err(error) = this.write(&pending)
            {
                tracing::warn!(%error, "pending chat delete record could not be cleared");
            }
        })
        .await;
    }
}

fn authorized(shared: &Shared, id: &str) -> bool {
    shared.temporary.ids().iter().any(|saved| saved == id)
        || shared.deletes.targets(id).is_some()
        || shared.confirmer.deletion_confirmed(id)
}

pub(crate) struct Prepared {
    targets: Targets,
    safe: io::Result<()>,
}

/// Capture storage before rmtree can remove a legacy folder's full-id metadata.
pub(crate) async fn prepare(shared: &Arc<Shared>, id: &str) -> Option<Prepared> {
    if !files::uuid(id) || !authorized(shared, id) {
        return None;
    }
    let mut targets = shared.deletes.targets(id).unwrap_or_else(|| Targets {
        save_dir: shared.deletes.configured_dir.clone(),
        prefix: shared.deletes.prefix.clone(),
        legacy: None,
    });
    let safe = {
        let (mut copy, id, configured) = (
            targets.clone(),
            id.to_owned(),
            shared.deletes.configured_dir.clone(),
        );
        match tokio::task::spawn_blocking(move || {
            let check = (|| {
                match files::root(&configured) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
                files::capture(&mut copy, &id)
            })();
            (copy, check)
        })
        .await
        {
            Ok((captured, result)) => {
                targets = captured;
                result
            }
            Err(error) => Err(io::Error::other(error)),
        }
    };
    // Reuse the send-admission fact, not just an empty transcript: this core created the id and
    // has never sent work (including an in-flight/cancelled turn). Absent, never-stored chats keep
    // their ordinary delete path and do not open another runtime merely to verify missing files.
    if shared.deletes.targets(id).is_none()
        && shared.plugin_refresh.unstarted(shared, id)
        && safe.is_ok()
        && targets.legacy.is_none()
    {
        let (copy, id) = (targets.clone(), id.to_owned());
        if matches!(
            tokio::task::spawn_blocking(move || files::has_unified(&copy, &id)).await,
            Ok(Ok(false))
        ) {
            return None;
        }
    }
    Some(Prepared { targets, safe })
}

/// Completes a delete that stopped part-way, so an interrupted delete never leaves a chat that cannot be deleted.
pub(crate) async fn finish(
    shared: &Arc<Shared>,
    id: &str,
    prepared: Option<Prepared>,
    result: Result<Reply, GateError>,
    guard: Option<OwnedMutexGuard<()>>,
) -> Result<Reply, GateError> {
    let Err(error) = result else {
        return result;
    };
    let eligible = files::uuid(id)
        && matches!(&error, GateError::Transport(RpcError::Remote { error, .. })
            if matches!(error.code.as_str(), "internal_error" | "not_found"));
    let Some(mut prepared) = prepared.filter(|_| eligible) else {
        return Err(error);
    };
    tracing::warn!(session_id = id, %error, "Vibe delete failed; finishing authorized chat deletion");
    let mut durable = shared.temporary.ids().iter().any(|saved| saved == id);
    if let Err(error) = shared.deletes.remember(id, prepared.targets.clone()).await {
        tracing::warn!(%error, "chat delete retry could not be saved");
    } else {
        durable = true;
    }
    if prepared.targets.save_dir != shared.deletes.configured_dir
        || prepared.targets.prefix != shared.deletes.prefix
    {
        prepared.safe = Err(io::Error::other(
            "configured chat storage changed; pending deletion retained",
        ));
    }
    let removed = match prepared.safe {
        Ok(()) => {
            let id = id.to_owned();
            tokio::task::spawn_blocking(move || {
                // A caller's timeout can drop this future while the OS is still removing files.
                // Keep the chat's serialization guard with that work until the blocking pass ends.
                let _held = guard;
                files::finish(&prepared.targets, &id)
            })
            .await
            .map_err(io::Error::other)
            .and_then(std::convert::identity)
        }
        Err(error) => Err(error),
    };
    match removed {
        Ok(()) => Ok(Reply {
            result: json!({}),
            client_user_message_id: None,
            user_entry_ids: Vec::new(),
        }),
        Err(cause) => {
            tracing::warn!(session_id = id, %cause, durable, "chat files remain; keeping deletion for the next start, without an immediate retry");
            let message = if durable {
                MESSAGE
            } else {
                "Vibe Work could not remove every file of this chat. Try Delete again."
            };
            Err(GateError::Transport(RpcError::Local {
                method: "session/delete".into(),
                source: Box::new(crate::CoreError::new(crate::ErrorCode::Internal, message)),
            }))
        }
    }
}

/// Retry each personal Delete once, before `Core::start` admits session readers.
pub(crate) async fn sweep(core: &Core) {
    for id in core.shared.deletes.ids() {
        if core.shared.temporary.ids().contains(&id) {
            continue;
        }
        let params = json!({"sessionId":id});
        if let Ok(token) = core
            .shared
            .gate
            .request_confirmation_as("session/delete", &params, vw_gate::Requester::Core)
            .await
        {
            let call = core.forward(
                crate::Origin::Ui,
                "session/delete",
                params,
                Some(token.as_str().to_owned()),
            );
            if !matches!(
                tokio::time::timeout(std::time::Duration::from_secs(10), call).await,
                Ok(Ok(_))
            ) {
                tracing::warn!(
                    session_id = id,
                    "chat deletion remains pending; retry at the next start"
                );
            }
        }
    }
}
