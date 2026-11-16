//! The knowledge base through a real core: the `knowledge` command, the app's own folders refused by the read
//! guards, and the list in the session facts every model turn carries.
//!
//! PREDICT: without the line in `app_context::append` that adds the knowledge text, a turn's `vw://app/context`
//! resource holds only the machine facts, so `a_turn_carries_the_places_and_drops_removed_ones` fails at its
//! first `contains`; with it, the places appear from the next turn on and a removed place is gone.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::command::{Command, Origin};
use crate::knowledge_view::{KnowledgeKind, KnowledgeRequest, KnowledgeView};

struct NoPrompt;
impl vw_gate::Confirmer for NoPrompt {
    async fn confirm(&self, _: &vw_gate::ConfirmationRequest) -> vw_gate::Confirmation {
        panic!("a knowledge base change never asks");
    }
}

/// A server-free core whose app data folder holds `settings.json`, with the Vibe Work note in place (the session
/// facts are only added when the note is ours).
async fn core(root: &std::path::Path) -> crate::Core {
    let data = root.join("app-data");
    let home = data.join("vibe");
    std::fs::create_dir_all(&home).unwrap();
    crate::working_note::write(&home, true).unwrap();
    let config = crate::CoreConfig {
        start_vibe: false,
        secret_scan_data: Some(data.clone()),
        agent_tools: Some(crate::AgentToolsConfig::new(
            root.join("no-tools-spawned"),
            data.join("tools"),
            None,
        )),
        ..crate::CoreConfig::default()
    };
    Box::pin(crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            root.join("no-vibe-spawned"),
            home,
            root.to_path_buf(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        config,
    ))
    .await
    .unwrap()
}

async fn knowledge(core: &crate::Core, request: KnowledgeRequest) -> Result<KnowledgeView, String> {
    match core.local(Origin::Ui, Command::Knowledge { request }).await {
        Ok(crate::CommandResult::Knowledge { view }) => Ok(view),
        Ok(_) => panic!("another result kind"),
        Err(error) => Err(error.message),
    }
}

async fn facts(core: &crate::Core) -> String {
    let mut params = json!({"sessionId": "s-knowledge", "message": [{"type": "text", "text": "look at our library"}]});
    crate::app_context::append(&core.shared, "turn/start", &mut params).await;
    params["message"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["resource"]["uri"] == "vw://app/context")
        .and_then(|part| part["resource"]["text"].as_str())
        .map(str::to_owned)
        .expect("the session facts resource")
}

fn folder(root: &std::path::Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn a_turn_carries_the_places_and_drops_removed_ones() {
    let scratch = tempfile::tempdir().unwrap();
    let root = vw_platform::canonical_plain(scratch.path()).unwrap();
    let core = core(&root).await;
    let library = folder(&root, "Library");
    assert!(!facts(&core).await.contains("Knowledge base"));
    let view = knowledge(
        &core,
        KnowledgeRequest::Add {
            name: "Our library".into(),
            kind: KnowledgeKind::Folder,
            location: library.to_str().unwrap().into(),
            label: None,
            note: Some("contracts and offers".into()),
        },
    )
    .await
    .unwrap();
    let text = facts(&core).await;
    assert!(
        text.contains("Knowledge base: places the user named."),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "- \"Our library\": folder {}. Note: \"contracts and offers\"",
            Value::String(library.to_str().unwrap().into())
        )),
        "{text}"
    );
    assert!(text.contains("gives no extra access"), "{text}");
    knowledge(
        &core,
        KnowledgeRequest::Remove {
            id: view.places[0].id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!facts(&core).await.contains("Our library"));
    core.shutdown().await;
}

#[tokio::test]
async fn the_app_data_folder_is_refused_and_a_list_survives_a_restart() {
    let scratch = tempfile::tempdir().unwrap();
    let root = vw_platform::canonical_plain(scratch.path()).unwrap();
    let first = core(&root).await;
    let refused = knowledge(
        &first,
        KnowledgeRequest::Add {
            name: "App data".into(),
            kind: KnowledgeKind::Folder,
            location: folder(&root, "app-data/tools").to_str().unwrap().into(),
            label: None,
            note: None,
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("never reads"), "{refused}");
    knowledge(
        &first,
        KnowledgeRequest::Add {
            name: "Team drive".into(),
            kind: KnowledgeKind::Drive,
            location: "Google Drive".into(),
            label: None,
            note: None,
        },
    )
    .await
    .unwrap();
    first.shutdown().await;
    let second = core(&root).await;
    let listed = knowledge(&second, KnowledgeRequest::List {}).await.unwrap();
    assert_eq!(listed.places.len(), 1);
    assert_eq!(listed.places[0].name, "Team drive");
    second.shutdown().await;
}

// PREDICT (red before the change): the folder check only refused a folder INSIDE a protected root, so the vault's
// container beside the app data (`Vibe Work Vault`, which holds `vault/` and its key file) and any folder above it
// were accepted; only the `vault/` folder itself and what is inside it were refused.
#[tokio::test]
async fn the_vault_folder_its_parents_and_its_contents_are_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let root = vw_platform::canonical_plain(scratch.path()).unwrap();
    let core = core(&root).await;
    // The same source the app uses for the vault's location on every OS: a sibling of the app data folder.
    let vault = vw_vault::private_location::private_vault_dir(&root.join("app-data")).unwrap();
    let key_store = vault.join("local-vault-server/repos/default");
    std::fs::create_dir_all(&key_store).unwrap();
    let container = vault.parent().unwrap().to_path_buf();
    for place in [&container, &root, &vault, &key_store] {
        let refused = knowledge(
            &core,
            KnowledgeRequest::Add {
                name: "Vault".into(),
                kind: KnowledgeKind::Folder,
                location: place.to_str().unwrap().into(),
                label: None,
                note: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            refused.contains("never reads"),
            "{}: {refused}",
            place.display()
        );
    }
    // A folder beside the vault and the app data stays a normal place.
    knowledge(
        &core,
        KnowledgeRequest::Add {
            name: "Library".into(),
            kind: KnowledgeKind::Folder,
            location: folder(&root, "library").to_str().unwrap().into(),
            label: None,
            note: None,
        },
    )
    .await
    .unwrap();
    core.shutdown().await;
}
