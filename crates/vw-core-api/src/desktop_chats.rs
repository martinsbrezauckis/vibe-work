//! Chats made in Vibe Desktop. Depends on `session_delete_files` (no-follow reads of Vibe's chat folder) and
//! the vocabulary; called by `session_listing` and the projects sweep (`projects_filing`; both hide them) and
//! `dispatch_forward` (refuses to open one).
//!
//! Invariants:
//! - Desktop stores its chats in the same `unified/<id>/` folders as Vibe Work, but stamps `meta.json` with
//!   `runtime_path: "session_api"` (Vibe Work's own chats say `direct`; older ones carry no field).
//! - Vibe 3.1.2 refuses Desktop's journal (its models forbid the extra `toolCallId`), so such a chat can never be
//!   opened here. It is therefore not listed, and an open by id is refused in plain words before Vibe is asked.
//! - Anything unreadable, linked, oversized or unstamped counts as not Desktop-made: the chat stays listed.

use std::io::Read;
use std::path::{Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;
use serde_json::Value;

use crate::core::Shared;
use crate::session_delete_files as files;
use crate::{CoreError, ErrorCode};

/// `serverCode` of the refusal.
pub(crate) const MADE_IN_DESKTOP: &str = "madeInDesktop";
const META_MAX: u64 = 256 * 1024;

/// The refusal for opening a Desktop-made chat.
pub(crate) fn refusal(method: &str) -> CoreError {
    let text = vw_vocabulary::texts::DESKTOP_CHAT_REFUSAL;
    let mut error = CoreError::new(ErrorCode::Denied, text).with_method(method);
    error.server_code = Some(MADE_IN_DESKTOP.to_owned());
    error
}

fn runtime_path(save_dir: &Path, id: &str) -> std::io::Result<Option<String>> {
    if !files::uuid(id) {
        return Ok(None);
    }
    let root = files::root(save_dir)?;
    let unified = files::child(&root, Path::new("unified"))?;
    let chat = files::child(&unified, Path::new(id))?;
    let name = Path::new("meta.json");
    let meta = chat.symlink_metadata(name)?;
    files::plain(&meta)?;
    if !meta.is_file() || meta.len() > META_MAX {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = chat.open_with(name, &options)?;
    files::plain(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.by_ref().take(META_MAX + 1).read_to_end(&mut bytes)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    Ok(value["runtime_path"].as_str().map(str::to_owned))
}

/// Whether the stored chat `id` was made in Vibe Desktop.
pub(crate) fn made_in_desktop(save_dir: &Path, id: &str) -> bool {
    matches!(runtime_path(save_dir, id), Ok(Some(path)) if path == "session_api")
}

/// Drops Desktop-made chats from a listing. One blocking pass over the folder reads, off the runtime threads.
pub(crate) async fn hide(save_dir: PathBuf, items: Vec<Value>) -> Vec<Value> {
    let fallback = items.clone();
    tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .filter(|item| {
                !item["id"]
                    .as_str()
                    .is_some_and(|id| made_in_desktop(&save_dir, id))
            })
            .collect()
    })
    .await
    // A failed pass hides nothing: an unreadable folder must not empty the list.
    .unwrap_or(fallback)
}

/// Refuses `session/resume`, `session/read` and `session/continue` for a Desktop-made chat before Vibe is asked.
pub(crate) fn check_open(shared: &Shared, method: &str, params: &Value) -> Result<(), CoreError> {
    check_in(shared.deletes.save_dir(), method, params)
}

fn check_in(save_dir: &Path, method: &str, params: &Value) -> Result<(), CoreError> {
    if matches!(
        method,
        "session/resume" | "session/read" | "session/continue"
    ) && params["sessionId"]
        .as_str()
        .is_some_and(|id| made_in_desktop(save_dir, id))
    {
        return Err(refusal(method));
    }
    Ok(())
}

/// Vibe's own validation text for a journal it cannot model, shown in place of the raw pydantic error.
pub(crate) fn friendly(method: &str, error: CoreError) -> CoreError {
    let opens = matches!(
        method,
        "session/resume" | "session/read" | "session/continue"
    );
    if opens
        && error.code == ErrorCode::ServerError
        && error.message.contains("Extra inputs are not permitted")
    {
        return refusal(method);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DESKTOP: &str = "9c9660d5-07c5-63f1-8e5a-966f77f09519";
    const OURS: &str = "6532e742-0000-4000-8000-000000000001";
    const OLD: &str = "6532e742-0000-4000-8000-000000000002";

    /// Folder shapes synthesized from the research note (no real chat content).
    fn folder() -> tempfile::TempDir {
        // The no-follow reader must not encounter macOS's /var -> /private/var temp symlink.
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let dir = tempfile::tempdir_in(temp).unwrap();
        for (id, meta) in [
            (
                DESKTOP,
                json!({"session_id":DESKTOP,"runtime_path":"session_api","project":{"kind":"local","id":"p"}}),
            ),
            (OURS, json!({"session_id":OURS,"runtime_path":"direct"})),
            (OLD, json!({"session_id":OLD})),
        ] {
            let chat = dir.path().join("unified").join(id);
            std::fs::create_dir_all(&chat).unwrap();
            std::fs::write(chat.join("meta.json"), meta.to_string()).unwrap();
        }
        // The journal shape that Vibe 3.1.2 refuses.
        let journal = dir.path().join("unified").join(DESKTOP).join("journal");
        std::fs::create_dir_all(&journal).unwrap();
        std::fs::write(
            journal.join("0.jsonl"),
            r#"{"type":"effect","title":"file_system.bash","toolCallId":"F8dkA9lPu"}"#,
        )
        .unwrap();
        dir
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fixture_folder_uses_canonical_temp_root() {
        let dir = folder();
        assert_eq!(dir.path(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn only_the_session_api_stamp_marks_a_desktop_chat() {
        let dir = folder();
        assert!(made_in_desktop(dir.path(), DESKTOP));
        assert!(!made_in_desktop(dir.path(), OURS));
        assert!(!made_in_desktop(dir.path(), OLD));
        assert!(!made_in_desktop(
            dir.path(),
            "6532e742-0000-4000-8000-0000000000ff"
        ));
        assert!(!made_in_desktop(dir.path(), "../etc"));
    }

    #[test]
    fn unreadable_storage_keeps_chats_listed() {
        let missing = Path::new("/nonexistent-vw-dir");
        assert!(!made_in_desktop(missing, DESKTOP));
    }

    /// Through the real entry points of a started core: the fixture lives in the core's own chat folder.
    #[tokio::test]
    async fn a_started_core_refuses_to_open_a_desktop_chat_without_asking_vibe() {
        let core = crate::offline_core::start("vdchats-open").await;
        let save = core.shared.deletes.save_dir().to_path_buf();
        let chat = save.join("unified").join(DESKTOP);
        std::fs::create_dir_all(&chat).unwrap();
        std::fs::write(chat.join("meta.json"), r#"{"runtime_path":"session_api"}"#).unwrap();
        let error = core
            .forward(
                crate::Origin::Ui,
                "session/resume",
                json!({"sessionId":DESKTOP}),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.server_code.as_deref(),
            Some(MADE_IN_DESKTOP),
            "{error:?}"
        );
        let _ = std::fs::remove_dir_all(save.join("unified").join(DESKTOP));
    }

    #[tokio::test]
    async fn the_listing_hides_desktop_chats_and_keeps_the_rest() {
        let dir = folder();
        let items = vec![
            json!({"id":DESKTOP}),
            json!({"id":OURS}),
            json!({"id":OLD}),
            json!({"title":"no id"}),
        ];
        let kept = hide(dir.path().to_path_buf(), items).await;
        let ids: Vec<_> = kept.iter().map(|i| i["id"].as_str()).collect();
        assert_eq!(ids, [Some(OURS), Some(OLD), None]);
    }

    #[test]
    fn opening_a_desktop_chat_is_refused_in_plain_words() {
        let dir = folder();
        for method in ["session/resume", "session/read", "session/continue"] {
            let error = check_in(dir.path(), method, &json!({"sessionId":DESKTOP})).unwrap_err();
            assert_eq!(error.server_code.as_deref(), Some(MADE_IN_DESKTOP));
            assert!(error.message.contains("Vibe Desktop"));
            assert!(!error.message.contains("pydantic") && !error.message.contains("Extra inputs"));
            assert!(check_in(dir.path(), method, &json!({"sessionId":OURS})).is_ok());
        }
        assert!(check_in(dir.path(), "turn/start", &json!({"sessionId":DESKTOP})).is_ok());
    }

    #[test]
    fn a_raw_validation_error_never_reaches_the_person() {
        let raw = CoreError::new(
            ErrorCode::ServerError,
            "1 validation error for Entry\neffect.toolCallId\n  Extra inputs are not permitted",
        );
        let shown = friendly("session/resume", raw.clone());
        assert_eq!(shown.server_code.as_deref(), Some(MADE_IN_DESKTOP));
        assert!(!shown.message.contains("Extra inputs"));
        // Other methods and other errors pass through untouched.
        assert_eq!(friendly("turn/start", raw.clone()), raw);
        let other = CoreError::new(ErrorCode::ServerError, "disk full");
        assert_eq!(friendly("session/resume", other.clone()), other);
    }
}
