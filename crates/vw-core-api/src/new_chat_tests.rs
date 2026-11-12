//! The New chat folder and a started chat's folder: never a Temporary chat's own folder, never a folder that is gone.
use serde_json::json;

use crate::{Command, CommandResult, ErrorCode, NewChatRequest, Origin};

async fn set_folder(core: &crate::Core, folder: &std::path::Path) -> Option<String> {
    // The Debug API's origin: the pick rule (`may_use`) lets it name any folder, so only the folder rule decides.
    let result = core
        .new_chat(
            Origin::DebugApi,
            NewChatRequest::SetFolder {
                folder: Some(folder.to_string_lossy().into_owned()),
            },
        )
        .await
        .unwrap();
    let CommandResult::NewChat { view } = result else {
        panic!("expected the draft view");
    };
    view.folder
}

#[tokio::test]
async fn a_temporary_chats_folder_is_never_the_new_chat_folder() {
    let core = crate::offline_core::start("new-chat-scratch-folder").await;
    let scratch = core.shared.temporary.new_scratch().unwrap();
    assert_eq!(set_folder(&core, &scratch).await, None);
    assert_eq!(core.shared.new_chat.lock().folder, None);
    // An ordinary folder still is.
    let work = core.shared.supervisor.work_dir().clone();
    assert_eq!(
        set_folder(&core, &work).await,
        Some(work.display().to_string())
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_folder_that_is_gone_is_dropped_not_kept() {
    let core = crate::offline_core::start("new-chat-gone-folder").await;
    let gone = core.shared.supervisor.work_dir().join("gone-folder");
    std::fs::create_dir_all(&gone).unwrap();
    assert!(set_folder(&core, &gone).await.is_some());
    std::fs::remove_dir(&gone).unwrap();
    // The restart case: the window names the folder it had; the core answers with no folder instead of a refusal.
    assert_eq!(set_folder(&core, &gone).await, None);
    assert!(!core.shared.new_chat.lock().refused);
    core.shutdown().await;
}

#[tokio::test]
async fn no_ordinary_chat_starts_in_a_temporary_or_missing_folder() {
    let core = crate::offline_core::start("new-chat-start-folder").await;
    let scratch = core.shared.temporary.new_scratch().unwrap();
    let gone = core.shared.supervisor.work_dir().join("never-made");
    for (cwd, says) in [
        (&scratch, "belongs to a temporary chat"),
        (&gone, "no longer exists"),
    ] {
        let config = json!({"cwd": cwd.to_string_lossy()});
        let commands = [
            Command::StartSession {
                agent_config: Some(config.clone()),
            },
            serde_json::from_value(json!({
                "type": "call", "method": "session/start", "params": {"agentConfig": config}
            }))
            .unwrap(),
        ];
        for command in commands {
            let error = core.execute(Origin::DebugApi, command).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::Denied, "{error:?}");
            assert!(error.message.contains(says), "{error:?}");
        }
    }
    core.shutdown().await;
}
