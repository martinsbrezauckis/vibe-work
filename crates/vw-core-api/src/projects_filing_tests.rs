//! Filing on a core without Vibe: the app's helper chats in Vibe's server working folder never make a project.
use serde_json::json;

fn projects(shared: &crate::core::Shared) -> Vec<(Option<String>, bool, usize)> {
    shared.projects.store.read(|s| {
        s.projects()
            .iter()
            .map(|p| (p.main_folder.clone(), p.auto, s.member_count(&p.id)))
            .collect()
    })
}

#[tokio::test]
async fn helper_chats_in_the_server_work_folder_are_never_filed() {
    let core = crate::offline_core::start("filing-work").await;
    let shared = &core.shared;
    let work = shared.supervisor.work_dir().to_string_lossy().into_owned();
    super::listed(
        shared,
        &[
            json!({"id":"helper-listed", "cwd": work}),
            json!({"id":"person", "cwd": "/code/app"}),
        ],
    )
    .await;
    let start = json!({"agentConfig": {"cwd": work}});
    let state = json!({"session": {"id": "helper-started", "cwd": work}});
    super::loaded(shared, &start, &state, true).await;
    let resumed = json!({"session": {"id": "helper-resumed", "cwd": format!("{work}/")}});
    super::loaded(shared, &json!({}), &resumed, false).await;
    let all = projects(shared);
    assert_eq!(
        all,
        vec![(Some("/code/app".to_owned()), true, 1)],
        "only the person's folder is a project: {all:?}"
    );
    for helper in ["helper-listed", "helper-started", "helper-resumed"] {
        assert!(
            shared
                .projects
                .store
                .read(|s| s.filing_of(helper).is_none()),
            "{helper} is not filed"
        );
    }
}

/// An older version made an automatic project for the work folder; once empty it is removed, never one a person
/// made or one with a chat.
#[tokio::test]
async fn the_empty_automatic_work_project_is_removed() {
    let core = crate::offline_core::start("filing-work-retire").await;
    let shared = &core.shared;
    let work = shared.supervisor.work_dir().to_string_lossy().into_owned();
    let seen = vec![vw_projects::Seen {
        id: "old-helper".into(),
        folder: Some(work.clone()),
    }];
    super::update(shared, move |s| {
        s.file_seen(&seen, 1);
        s.forget("old-helper");
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(projects(shared).len(), 1);
    super::retire_work_project(shared).await;
    assert_eq!(projects(shared), Vec::new());
}

/// PREDICT (before the change): a Temporary chat started without a folder shows in the sidebar under a project
/// named after its own random folder (`chat-<16 hex>`). After it the project reads "Temporary chat" (no folder name),
/// whether the start or a later listing filed the chat first; a chat in a person's folder keeps its folder name.
/// PREDICT (one group): each Temporary chat's folder got a project of its own, "Temporary chat", "Temporary chat (2)",
/// ... After it every Temporary chat sits in one project, "Temporary chats".
#[tokio::test]
async fn a_temporary_chats_own_folder_shows_as_temporary_chat() {
    let core = crate::offline_core::start("filing-temporary").await;
    let shared = &core.shared;
    let names = || -> Vec<String> {
        shared
            .projects
            .store
            .read(|s| s.projects().iter().map(|p| p.name.clone()).collect())
    };
    let started = shared.temporary.new_scratch().unwrap();
    let started = started.to_string_lossy().into_owned();
    let start = json!({"agentConfig": {"cwd": started}});
    let state = json!({"session": {"id": "temporary-started", "cwd": started}});
    super::loaded(shared, &start, &state, true).await;
    let listed = shared.temporary.new_scratch().unwrap();
    super::listed(shared, &[json!({"id":"temporary-listed", "cwd": listed})]).await;
    super::loaded(
        shared,
        &json!({}),
        &json!({"session": {"id": "person", "cwd": "/code/app"}}),
        false,
    )
    .await;
    let all = names();
    assert_eq!(all, vec!["Temporary chats", "app"], "{all:?}");
    for dir in [std::path::PathBuf::from(started), listed] {
        shared.temporary.remove_scratch(&dir);
    }
    core.shutdown().await;
}

/// PREDICT (before the change): a "Temporary chat" project outlives its chat. When the chat is gone but still filed
/// in the project (its delete ended without the project being told, for example Vibe answered "not found" at the
/// next start) and its folder is gone too, the cleanup sweep at quit or start leaves the empty project in the
/// sidebar forever: the sweep retires only projects of folders it still finds on disk, and retiring refuses a
/// project that has a member. After it, once no temporary chat is left, every Temporary chat project goes with its
/// stale members, while a person's project and its chat stay.
#[tokio::test]
async fn a_temporary_chats_project_goes_once_no_temporary_chat_is_left() {
    let core = crate::offline_core::start("filing-temporary-leftover").await;
    let shared = &core.shared;
    let names = || -> Vec<String> {
        shared
            .projects
            .store
            .read(|s| s.projects().iter().map(|p| p.name.clone()).collect())
    };
    let gone = shared.temporary.new_scratch().unwrap();
    let gone_folder = gone.to_string_lossy().into_owned();
    super::loaded(
        shared,
        &json!({"agentConfig": {"cwd": gone_folder}}),
        &json!({"session": {"id": "gone-temporary", "cwd": gone_folder}}),
        true,
    )
    .await;
    // Its folder is already gone (removed with the chat at an earlier quit).
    shared.temporary.remove_scratch(&gone);
    assert!(!gone.exists());
    // A folder of a temporary chat that is still on disk goes with the sweep as before.
    let kept = shared.temporary.new_scratch().unwrap();
    super::listed(
        shared,
        &[json!({"id": "left-temporary", "cwd": kept.to_string_lossy()})],
    )
    .await;
    super::loaded(
        shared,
        &json!({}),
        &json!({"session": {"id": "person", "cwd": "/code/app"}}),
        false,
    )
    .await;
    assert_eq!(names(), vec!["Temporary chats", "app"], "{:?}", names());
    core.delete_temporary_all(true).await;
    assert_eq!(names(), vec!["app"]);
    assert!(!kept.exists());
    let filed = |id: &str| shared.projects.store.read(|s| s.filing_of(id).is_some());
    assert!(filed("person"), "a person's chat stays filed");
    assert!(!filed("gone-temporary") && !filed("left-temporary"));
    core.shutdown().await;
}

/// PREDICT (before the change): every Temporary chat's own folder gets its own project, so three Temporary chats show as
/// "Temporary chat", "Temporary chat (2)" and "Temporary chat (3)". After it they share one project, "Temporary chats",
/// which stays while any of them is left (also when the chat whose folder it was made for ends first) and goes with
/// the last one.
#[tokio::test]
async fn every_temporary_chat_sits_in_one_temporary_chats_project() {
    let core = crate::offline_core::start("filing-temporary-one").await;
    let shared = &core.shared;
    let names = || -> Vec<String> {
        shared
            .projects
            .store
            .read(|s| s.projects().iter().map(|p| p.name.clone()).collect())
    };
    let project_of = |id: &str| {
        shared
            .projects
            .store
            .read(|s| s.filing_of(id).flatten().map(str::to_owned))
    };
    let mut ids = Vec::new();
    for n in 0..3 {
        let dir = shared.temporary.new_scratch().unwrap();
        let cwd = dir.to_string_lossy().into_owned();
        let id = format!("temporary-{n}");
        if n == 1 {
            // One filed by a listing, the others by their start.
            super::listed(shared, &[json!({"id": id, "cwd": cwd})]).await;
        } else {
            super::loaded(
                shared,
                &json!({"agentConfig": {"cwd": cwd}}),
                &json!({"session": {"id": id, "cwd": cwd}}),
                true,
            )
            .await;
        }
        shared.temporary.bind_scratch(&id, dir);
        ids.push(id);
    }
    super::loaded(
        shared,
        &json!({}),
        &json!({"session": {"id": "person", "cwd": "/code/app"}}),
        false,
    )
    .await;
    assert_eq!(names(), vec!["Temporary chats", "app"], "{:?}", names());
    let shared_project = project_of("temporary-0");
    assert!(shared_project.is_some());
    for id in &ids {
        assert_eq!(project_of(id), shared_project, "{id}");
    }
    // The chat whose folder the project was made for ends first: the others keep the project.
    for (at, id) in ids.iter().enumerate() {
        super::deleted(shared, id).await;
        crate::temporary::chat_ended(shared, id).await;
        let left = if at + 1 < ids.len() {
            vec!["Temporary chats", "app"]
        } else {
            vec!["app"]
        };
        assert_eq!(names(), left, "after {id} ended: {:?}", names());
    }
    core.shutdown().await;
}
