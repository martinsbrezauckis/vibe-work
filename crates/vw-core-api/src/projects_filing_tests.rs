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
