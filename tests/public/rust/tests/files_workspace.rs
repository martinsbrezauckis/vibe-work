//! Folder browsing, file opening and commits before a session exists.
//! Uses the server-free core and disposable repositories; run by the public Rust gate.
mod changes;
mod common;
use changes::TempRepo;
use common::core::{ScriptRig, no_vibe_core_with as script_core_with};
use serde_json::{Value, json};
use vw_core_api::{CommandResult, CoreError, Origin};
async fn run(rig: &ScriptRig, body: Value) -> Result<CommandResult, CoreError> {
    rig.core
        .execute(Origin::Ui, serde_json::from_value(body).unwrap())
        .await
}
#[tokio::test]
async fn chosen_folder_opens_files_and_commits_without_starting_a_session() {
    let repo = TempRepo::with_files(&[("notes.txt", "before\n")]);
    let data = repo.data.clone();
    let rig = script_core_with("folder-without-session", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    assert!(
        rig.core.status().server.pid.is_none(),
        "folder-only checks must start no Vibe"
    );
    let folder = repo.root.to_string_lossy().into_owned();
    let request = json!({"type":"gitBrowser","request":{"op":"explore","folder":folder,"request":{"op":"folder","path":"","showIgnored":false,"offset":0}}});
    assert!(run(&rig, request.clone()).await.is_err());
    rig.core.register_picked_folder(&repo.root).unwrap();
    let result = run(&rig, request).await.unwrap();
    assert!(
        serde_json::to_value(result)
            .unwrap()
            .to_string()
            .contains("notes.txt")
    );
    let workspace = format!("files-workspace:{folder}");
    let result=run(&rig,json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["notes.txt"]}})).await.unwrap();
    let result = serde_json::to_value(result).unwrap();
    let file = result["result"]["refs"][0]["file"].as_str().unwrap();
    let read = run(
        &rig,
        json!({"type":"files","request":{"op":"read","sessionId":workspace,"file":file}}),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(read).unwrap()["result"]["content"]["text"],
        "before\n"
    );
    repo.write("notes.txt", "after\n");
    let result = run(
        &rig,
        json!({"type":"git","request":{"op":"prepareCommit","sessionId":workspace,"repo":"."}}),
    )
    .await
    .unwrap();
    let result = serde_json::to_value(result).unwrap();
    let plan = &result["result"]["plan"];
    assert_eq!(plan["files"][0]["path"], "notes.txt");
    run(&rig,json!({"type":"git","request":{"op":"commit","sessionId":workspace,"repo":".","paths":["notes.txt"],"message":"Explain the folder change","expectedHead":plan["head"],"branch":plan["branch"],"runHooks":false}})).await.unwrap();
    assert!(
        repo.git(&["log", "-1", "--format=%s"])
            .contains("Explain the folder change")
    );
    assert!(rig.core.status().server.pid.is_none());
    assert!(rig.core.status().loaded_sessions.is_empty());
    assert!(rig.received().is_empty(), "no Vibe request was needed");
    rig.core.shutdown().await;
}
/// A folder lexically inside a known root must still resolve inside it: `..` hops are cleaned before the
/// membership check, so `<picked>/..` (or a workspace key built from it) never reaches the parent.
#[tokio::test]
async fn parent_hops_out_of_a_known_folder_are_refused() {
    let repo =
        TempRepo::with_files(&[("outside.txt", "secret\n"), ("inner/notes.txt", "inside\n")]);
    let data = repo.data.clone();
    let rig = script_core_with("folder-parent-hop", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    rig.core
        .register_picked_folder(&repo.root.join("inner"))
        .unwrap();
    let inner = repo.root.join("inner").to_string_lossy().into_owned();
    let hop = format!("{inner}/..");
    let explore = |folder: &str| json!({"type":"gitBrowser","request":{"op":"explore","folder":folder,"request":{"op":"folder","path":"","showIgnored":false,"offset":0}}});
    let inside = run(&rig, explore(&inner)).await.unwrap();
    assert!(
        serde_json::to_value(inside)
            .unwrap()
            .to_string()
            .contains("notes.txt")
    );
    assert!(
        run(&rig, explore(&hop)).await.is_err(),
        "a parent hop listed the folder above the picked one"
    );
    let workspace = format!("files-workspace:{hop}");
    let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["outside.txt"]}})).await;
    let text = resolved
        .map(|r| serde_json::to_value(r).unwrap().to_string())
        .unwrap_or_default();
    assert!(
        !text.contains("\"file\""),
        "a workspace key with a parent hop resolved a file outside the picked folder: {text}"
    );
    rig.core.shutdown().await;
}

/// The window names a picked folder as the folder dialog gave it (Windows: `C:\...`), while the gate keeps the
/// canonical spelling (Windows: `\\?\C:\...`). In either spelling, before any session exists, the folder lists,
/// its file opens and saves, and a search in it runs. Nothing beside it opens: a sibling whose name starts the
/// same, the folder above, and (Windows) the same names in another case, on another drive or as a network share.
#[tokio::test]
async fn a_picked_folder_opens_in_the_dialog_spelling_and_nothing_beside_it_does() {
    let repo = TempRepo::with_files(&[
        ("inner/notes.txt", "before\n"),
        ("inner-x/other.txt", "other\n"),
        ("outside.txt", "outside\n"),
    ]);
    let data = repo.data.clone();
    let rig = script_core_with("folder-dialog-spelling", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    let canonical = repo.root.join("inner");
    let dialog = vw_platform::plain_path(&canonical);
    if cfg!(windows) {
        assert_ne!(dialog, canonical, "the canonical form should carry \\\\?\\");
    }
    rig.core.register_picked_folder(&dialog).unwrap();
    let explore = |folder: &str| json!({"type":"gitBrowser","request":{"op":"explore","folder":folder,"request":{"op":"folder","path":"","showIgnored":false,"offset":0}}});
    for spelling in [&dialog, &canonical] {
        let folder = spelling.to_string_lossy().into_owned();
        let listed = run(&rig, explore(&folder)).await.unwrap();
        assert!(
            serde_json::to_value(listed)
                .unwrap()
                .to_string()
                .contains("notes.txt"),
            "{folder} did not list"
        );
    }
    // Open and save through the workspace key the window builds from the dialog's spelling.
    let workspace = format!("files-workspace:{}", dialog.display());
    let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["notes.txt"]}})).await.unwrap();
    let resolved = serde_json::to_value(resolved).unwrap();
    let file = resolved["result"]["refs"][0]["file"]
        .as_str()
        .unwrap_or_else(|| panic!("notes.txt not resolved: {resolved}"))
        .to_owned();
    let read = run(
        &rig,
        json!({"type":"files","request":{"op":"read","sessionId":workspace,"file":file}}),
    )
    .await
    .unwrap();
    let read = serde_json::to_value(read).unwrap();
    assert_eq!(read["result"]["content"]["text"], "before\n");
    let print = read["result"]["content"]["fingerprint"].clone();
    run(&rig, json!({"type":"files","request":{"op":"write","sessionId":workspace,"file":file,"text":"after\n","fingerprint":print}})).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(canonical.join("notes.txt")).unwrap(),
        "after\n"
    );
    let search = run(&rig, json!({"type":"search","request":{"op":"files","query":"notes","namesOnly":true,"folder":dialog.to_string_lossy()}})).await;
    let search = serde_json::to_value(search.unwrap()).unwrap().to_string();
    assert!(search.contains("notes.txt"), "{search}");

    let mut refused = vec![
        repo.root.join("inner-x"),
        dialog.join(".."),
        canonical.join(".."),
    ];
    if cfg!(windows) {
        let text = dialog.to_string_lossy().into_owned();
        refused.push(std::path::PathBuf::from(text.to_uppercase()));
        refused.push(std::path::PathBuf::from(text.to_lowercase()));
        let drive = if text.starts_with(['Z', 'z']) {
            'Y'
        } else {
            'Z'
        };
        refused.push(std::path::PathBuf::from(format!("{drive}{}", &text[1..])));
        refused.push(std::path::PathBuf::from(format!(
            r"\\127.0.0.1\{}${}",
            &text[..1],
            &text[2..]
        )));
        refused.push(std::path::PathBuf::from(format!(
            r"\\?\UNC\127.0.0.1\{}${}",
            &text[..1],
            &text[2..]
        )));
    }
    for path in refused {
        let folder = path.to_string_lossy().into_owned();
        // On a case-insensitive volume the other-case spellings name the same folder; they are refused anyway,
        // because a folder can be case-sensitive on Windows too and then they name another one.
        assert!(
            run(&rig, explore(&folder)).await.is_err(),
            "{folder} was listed"
        );
        let workspace = format!("files-workspace:{folder}");
        let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["outside.txt", "other.txt", "notes.txt"]}})).await;
        let text = resolved
            .map(|r| serde_json::to_value(r).unwrap().to_string())
            .unwrap_or_default();
        assert!(
            !text.contains("\"file\""),
            "{folder} resolved a file: {text}"
        );
        let search = run(&rig, json!({"type":"search","request":{"op":"files","query":"txt","namesOnly":true,"folder":folder}})).await;
        assert!(search.is_err(), "{folder} was searched: {search:?}");
    }
    rig.core.shutdown().await;
}

/// A folder chosen through a symbolic link (macOS: `/tmp/x` is
/// `/private/tmp/x`, the temp folder `/var/...` is `/private/var/...`; anywhere: a linked project folder) keeps the
/// link spelling in the window, while the gate keeps the folder it leads to. In the link spelling, before any session
/// exists, the folder lists, its file opens and saves, and a search in it runs; a project whose main folder is the
/// link spelling lists after a restart (no pick in that run). Nothing outside opens: a link inside the folder that
/// leads out of it (in either spelling, also as a file path), `..` above the folder, and another link to the outside
/// folder that was never chosen.
#[cfg(unix)]
#[tokio::test]
async fn path2_a_folder_chosen_through_a_symbolic_link_opens_and_links_out_of_it_stay_refused() {
    let repo = TempRepo::with_files(&[
        ("inner/notes.txt", "before\n"),
        ("outside/secret.txt", "secret\n"),
    ]);
    let canonical = repo.root.join("inner");
    std::os::unix::fs::symlink(repo.root.join("outside"), canonical.join("escape")).unwrap();
    // The spelling the folder dialog gives: a link beside the repository that leads to its `inner` folder.
    let base = repo.data.parent().unwrap().to_path_buf();
    let link = base.join("linked-project");
    std::os::unix::fs::symlink(&canonical, &link).unwrap();
    let stray = base.join("stray-link");
    std::os::unix::fs::symlink(repo.root.join("outside"), &stray).unwrap();
    let data = repo.data.clone();
    let projects = base.join("projects");
    let first_projects = projects.clone();
    let rig = script_core_with("folder-through-link", &[], move |config| {
        config.changes_dir = Some(data);
        config.projects_dir = Some(first_projects);
    })
    .await;
    rig.core.register_picked_folder(&link).unwrap();
    let explore = |folder: &str| json!({"type":"gitBrowser","request":{"op":"explore","folder":folder,"request":{"op":"folder","path":"","showIgnored":false,"offset":0}}});
    for spelling in [&link, &canonical] {
        let folder = spelling.to_string_lossy().into_owned();
        let listed = run(&rig, explore(&folder)).await;
        let listed =
            serde_json::to_value(listed.unwrap_or_else(|e| panic!("{folder}: {e:?}"))).unwrap();
        assert!(
            listed.to_string().contains("notes.txt"),
            "{folder} did not list: {listed}"
        );
    }
    let workspace = format!("files-workspace:{}", link.display());
    let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["notes.txt"]}})).await.unwrap();
    let resolved = serde_json::to_value(resolved).unwrap();
    let file = resolved["result"]["refs"][0]["file"]
        .as_str()
        .unwrap_or_else(|| panic!("notes.txt not resolved: {resolved}"))
        .to_owned();
    let read = run(
        &rig,
        json!({"type":"files","request":{"op":"read","sessionId":workspace,"file":file}}),
    )
    .await
    .unwrap();
    let read = serde_json::to_value(read).unwrap();
    assert_eq!(read["result"]["content"]["text"], "before\n");
    let print = read["result"]["content"]["fingerprint"].clone();
    run(&rig, json!({"type":"files","request":{"op":"write","sessionId":workspace,"file":file,"text":"after\n","fingerprint":print}})).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(canonical.join("notes.txt")).unwrap(),
        "after\n"
    );
    let search = run(&rig, json!({"type":"search","request":{"op":"files","query":"notes","namesOnly":true,"folder":link.to_string_lossy()}})).await;
    let search = serde_json::to_value(search.unwrap()).unwrap().to_string();
    assert!(search.contains("notes.txt"), "{search}");
    // Links that lead out of the folder: nothing lists, resolves or searches.
    let escape_paths = [
        link.join("escape"),
        canonical.join("escape"),
        link.join(".."),
        stray.clone(),
    ];
    for path in &escape_paths {
        let folder = path.to_string_lossy().into_owned();
        assert!(
            run(&rig, explore(&folder)).await.is_err(),
            "{folder} was listed"
        );
        let key = format!("files-workspace:{folder}");
        let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":key,"paths":["secret.txt", "notes.txt"]}})).await;
        let text = resolved
            .map(|r| serde_json::to_value(r).unwrap().to_string())
            .unwrap_or_default();
        assert!(
            !text.contains("\"file\":\""),
            "{folder} resolved a file: {text}"
        );
        let search = run(&rig, json!({"type":"search","request":{"op":"files","query":"secret","namesOnly":true,"folder":folder}})).await;
        assert!(search.is_err(), "{folder} was searched: {search:?}");
    }
    let through = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["escape/secret.txt"]}})).await;
    let through = through
        .map(|r| serde_json::to_value(r).unwrap().to_string())
        .unwrap_or_default();
    assert!(
        !through.contains("\"file\":\""),
        "a file behind the inner link resolved: {through}"
    );
    let inside = run(&rig, json!({"type":"gitBrowser","request":{"op":"explore","folder":link.to_string_lossy(),"request":{"op":"folder","path":"escape","showIgnored":false,"offset":0}}})).await;
    let inside = inside
        .map(|r| serde_json::to_value(r).unwrap().to_string())
        .unwrap_or_default();
    assert!(
        !inside.contains("secret.txt"),
        "the inner link was listed: {inside}"
    );

    // A project keeps the link spelling as its main folder; after a restart (nothing picked) it still lists.
    let made = run(&rig, json!({"type":"projects","request":{"op":"create","name":"Linked","mainFolder":link.to_string_lossy()}})).await;
    made.unwrap();
    rig.core.shutdown().await;
    let data = repo.data.clone();
    let rig = script_core_with("folder-through-link-again", &[], move |config| {
        config.changes_dir = Some(data);
        config.projects_dir = Some(projects);
    })
    .await;
    let listed = run(&rig, explore(&link.to_string_lossy())).await;
    let listed = serde_json::to_value(
        listed.unwrap_or_else(|e| panic!("project folder after a restart: {e:?}")),
    )
    .unwrap();
    assert!(listed.to_string().contains("notes.txt"), "{listed}");
    assert!(
        run(&rig, explore(&link.join("escape").to_string_lossy()))
            .await
            .is_err()
    );
    rig.core.shutdown().await;
}

/// The New chat screen shows the folder it had before a restart as the
/// chosen folder; a folder the user picked for it must then still work: the Files tab lists it and a file in it
/// opens, with no new pick. Exactly that folder and only while it is the New chat folder: another folder picked in
/// the earlier run, a folder inside the restored one named as the New chat folder, and the restored folder after the
/// screen moved to another folder all stay refused; the folder is not trusted for anything else (not picked); and a
/// folder whose name now leads somewhere else (replaced by a link) is refused.
#[tokio::test]
async fn path2_the_restored_new_chat_folder_works_after_a_restart_and_nothing_else_does() {
    let repo = TempRepo::with_files(&[
        ("inner/notes.txt", "before\n"),
        ("inner/sub/deep.txt", "deep\n"),
        ("other/other.txt", "other\n"),
    ]);
    let base = repo.data.parent().unwrap().to_path_buf();
    let sessions = base.join("sessions");
    let folder = repo.root.join("inner");
    let other = repo.root.join("other");
    let text = folder.to_string_lossy().into_owned();
    let start = |name: &'static str| {
        let data = repo.data.clone();
        let sessions = sessions.clone();
        script_core_with(name, &[], move |config| {
            config.changes_dir = Some(data);
            config.sessions_dir = Some(sessions);
        })
    };
    let set_folder = |folder: Option<&str>| match folder {
        Some(folder) => json!({"type":"newChat","request":{"op":"setFolder","folder":folder}}),
        None => json!({"type":"newChat","request":{"op":"setFolder"}}),
    };
    let explore = |folder: &str| json!({"type":"gitBrowser","request":{"op":"explore","folder":folder,"request":{"op":"folder","path":"","showIgnored":false,"offset":0}}});
    let lists = |result: Result<CommandResult, CoreError>| {
        result.is_ok_and(|r| {
            serde_json::to_value(r)
                .unwrap()
                .to_string()
                .contains(".txt")
        })
    };

    // First run: both folders picked, `inner` becomes the New chat folder.
    let rig = start("restored-new-chat-1").await;
    rig.core.register_picked_folder(&folder).unwrap();
    rig.core.register_picked_folder(&other).unwrap();
    run(&rig, set_folder(Some(&text))).await.unwrap();
    assert!(lists(run(&rig, explore(&text)).await));
    rig.core.shutdown().await;

    // After a restart the window names the same folder again (nothing picked in this run).
    let rig = start("restored-new-chat-2").await;
    let restored = run(&rig, set_folder(Some(&text))).await;
    assert!(
        restored.is_ok(),
        "the restored New chat folder was refused: {restored:?}"
    );
    assert!(
        lists(run(&rig, explore(&text)).await),
        "the Files tab of the restored folder was refused"
    );
    let workspace = format!("files-workspace:{text}");
    let resolved = run(&rig, json!({"type":"files","request":{"op":"resolve","sessionId":workspace,"paths":["notes.txt"]}})).await.unwrap();
    let resolved = serde_json::to_value(resolved).unwrap();
    let file = resolved["result"]["refs"][0]["file"]
        .as_str()
        .unwrap_or_else(|| panic!("notes.txt not resolved: {resolved}"))
        .to_owned();
    let read = run(
        &rig,
        json!({"type":"files","request":{"op":"read","sessionId":workspace,"file":file}}),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(read).unwrap()["result"]["content"]["text"],
        "before\n"
    );
    // Not trusted for anything else, and the other folder of the earlier run is not restored.
    assert!(!rig.core.is_picked_folder(&folder));
    assert!(!lists(run(&rig, explore(&other.to_string_lossy())).await));
    rig.core.shutdown().await;

    // A folder inside the restored one, named as the New chat folder after a restart, is not the restored folder.
    let rig = start("restored-new-chat-3").await;
    let sub = folder.join("sub").to_string_lossy().into_owned();
    assert!(run(&rig, set_folder(Some(&sub))).await.is_err());
    assert!(!lists(run(&rig, explore(&sub)).await));
    // The window names the restored folder: it works; then the screen moves away and back: refused.
    run(&rig, set_folder(Some(&text))).await.unwrap();
    assert!(lists(run(&rig, explore(&text)).await));
    run(&rig, set_folder(None)).await.unwrap();
    assert!(
        !lists(run(&rig, explore(&text)).await),
        "listed after the screen left the folder"
    );
    assert!(run(&rig, set_folder(Some(&text))).await.is_err());
    rig.core.shutdown().await;

    // The record went with the folder: a restart restores nothing now.
    let rig = start("restored-new-chat-4").await;
    assert!(run(&rig, set_folder(Some(&text))).await.is_err());
    // Picked again: restored again after a restart, unless the name now leads somewhere else.
    rig.core.register_picked_folder(&folder).unwrap();
    run(&rig, set_folder(Some(&text))).await.unwrap();
    rig.core.shutdown().await;
    let moved = repo.root.join("inner-moved");
    std::fs::rename(&folder, &moved).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, &folder).unwrap();
    // Windows: a junction, which any user may make (a symbolic link needs a privilege).
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&folder)
            .arg(&other)
            .status()
            .unwrap()
            .success()
    );
    let rig = start("restored-new-chat-5").await;
    assert!(
        run(&rig, set_folder(Some(&text))).await.is_err(),
        "a folder replaced since the pick was restored"
    );
    assert!(!lists(run(&rig, explore(&text)).await));
    rig.core.shutdown().await;
}
