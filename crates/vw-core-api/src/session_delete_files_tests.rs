//! Deletion boundary tests with real files, links and legacy short-id collisions.
//! Calls the handle walker directly; integration tests exercise consent and real Vibe failure ordering.
use super::*;

const ID: &str = "11111111-2222-3333-4444-555555555555";
const OTHER: &str = "11111111-2222-3333-4444-666666666666";

fn fixture() -> (PathBuf, Targets) {
    let dir = std::env::temp_dir().join(format!(
        "vw-tmpdel-files-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("save/unified").join(ID)).unwrap();
    let dir = vw_platform::plain_path(&dir.canonicalize().unwrap());
    let targets = Targets {
        save_dir: dir.join("save"),
        prefix: "session".into(),
        legacy: None,
    };
    std::fs::write(
        targets.save_dir.join("unified").join(ID).join("chat"),
        b"private",
    )
    .unwrap();
    (dir, targets)
}

fn clean(dir: &Path) {
    vw_platform::remove_owned_tree(dir.parent().unwrap(), dir).unwrap();
}

#[test]
fn exact_uuid_only_and_save_directory_survives() {
    let (dir, targets) = fixture();
    for id in [
        "",
        ".",
        "..",
        "unified",
        "../save",
        "11111111-2222-3333-4444-55555555555/",
        "11111111222233334444555555555555",
    ] {
        assert!(finish(&targets, id).is_err(), "accepted {id}");
    }
    assert!(targets.save_dir.join("unified").join(ID).exists());
    for name in [
        "../session_x_11111111",
        "session_x:stream_11111111",
        "session_x\\bad_11111111",
    ] {
        let mut unsafe_target = targets.clone();
        unsafe_target.legacy = Some(name.into());
        assert!(finish(&unsafe_target, ID).is_err());
        assert!(targets.save_dir.join("unified").join(ID).exists());
    }
    finish(&targets, ID).unwrap();
    finish(&targets, ID).unwrap();
    assert!(targets.save_dir.join("unified").is_dir());
    clean(&dir);
}

#[test]
fn legacy_matches_the_full_id_and_survives_metadata_unlinking() {
    for prefix in ["session", ""] {
        let (dir, mut targets) = fixture();
        targets.prefix = prefix.into();
        let wrong = targets.save_dir.join(format!("{prefix}_20260101_11111111"));
        let right = targets.save_dir.join(format!("{prefix}_20260102_11111111"));
        for (folder, id) in [(&wrong, OTHER), (&right, ID)] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(
                folder.join("meta.json"),
                serde_json::json!({"session_id":id}).to_string(),
            )
            .unwrap();
            std::fs::write(folder.join("messages.jsonl"), "private").unwrap();
        }
        capture(&mut targets, ID).unwrap();
        assert_eq!(
            targets.legacy.as_deref(),
            Some(format!("{prefix}_20260102_11111111").as_str())
        );
        std::fs::remove_file(right.join("meta.json")).unwrap();
        finish(&targets, ID).unwrap();
        assert!(!right.exists());
        assert_eq!(
            std::fs::read_to_string(wrong.join("messages.jsonl")).unwrap(),
            "private"
        );
        clean(&dir);
    }
}

#[cfg(unix)]
#[test]
fn links_at_each_boundary_and_inside_are_refused_without_removing_any_tree() {
    use std::os::unix::fs::symlink;
    for place in ["ancestor", "save", "unified", "target", "inside"] {
        let (dir, mut targets) = fixture();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), "outside").unwrap();
        let target = targets.save_dir.join("unified").join(ID);
        let link = match place {
            "ancestor" => {
                symlink(&dir, dir.join("alias")).unwrap();
                targets.save_dir = dir.join("alias/save");
                None
            }
            "save" => Some(targets.save_dir.clone()),
            "unified" => Some(targets.save_dir.join("unified")),
            "target" => Some(target.clone()),
            "inside" => Some(target.join("link")),
            _ => unreachable!(),
        };
        if let Some(link) = link {
            if link.exists() {
                std::fs::rename(&link, dir.join("original")).unwrap();
            }
            symlink(&outside, &link).unwrap();
        }
        assert!(finish(&targets, ID).is_err(), "accepted {place}");
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "outside"
        );
        if place == "inside" {
            assert!(
                target.join("chat").exists(),
                "preflight must precede all removals"
            );
        }
        clean(&dir);
    }
}

#[cfg(windows)]
#[test]
fn junction_at_target_or_inside_is_refused() {
    for place in ["ancestor", "save", "unified", "target", "inside"] {
        let (dir, mut targets) = fixture();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), "outside").unwrap();
        let target = targets.save_dir.join("unified").join(ID);
        let link = match place {
            "ancestor" => {
                targets.save_dir = dir.join("alias/save");
                dir.join("alias")
            }
            "save" => targets.save_dir.clone(),
            "unified" => targets.save_dir.join("unified"),
            "target" => target,
            "inside" => target.join("junction"),
            _ => unreachable!(),
        };
        let destination = if place == "ancestor" { &dir } else { &outside };
        if link.exists() {
            std::fs::rename(&link, dir.join("original")).unwrap();
        }
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(destination)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(finish(&targets, ID).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "outside"
        );
        clean(&dir);
    }
}

/// Vibe keeps each chat's plugin data in `<save>/plugins/data/<id>/<plugin>` and never removes it: the chat's own
/// folder goes with the chat; other chats' folders and the shared folders stay.
#[test]
fn the_chats_plugin_data_folder_goes_with_it() {
    let (dir, targets) = fixture();
    let data = targets.save_dir.join("plugins/data");
    for id in [ID, OTHER] {
        std::fs::create_dir_all(data.join(id).join("plugin/cache")).unwrap();
        std::fs::write(data.join(id).join("plugin/cache/state"), id).unwrap();
    }
    std::fs::create_dir_all(data.join(ID).join("empty")).unwrap();
    finish(&targets, ID).unwrap();
    assert!(!targets.save_dir.join("unified").join(ID).exists());
    assert!(!data.join(ID).exists(), "the chat's plugin data stays");
    assert_eq!(
        std::fs::read_to_string(data.join(OTHER).join("plugin/cache/state")).unwrap(),
        OTHER
    );
    // No plugin folder at all is fine too.
    let (bare, targets) = fixture();
    finish(&targets, ID).unwrap();
    assert!(!targets.save_dir.join("unified").join(ID).exists());
    clean(&dir);
    clean(&bare);
}

/// Plugin data is the plugins' own: a link in it (or at it) is never followed or removed, and it does not stop the
/// chat's own deletion.
#[cfg(unix)]
#[test]
fn a_link_in_plugin_data_is_left_and_never_followed() {
    use std::os::unix::fs::symlink;
    for place in ["plugins", "data", "chat", "inside"] {
        let (dir, targets) = fixture();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), "outside").unwrap();
        let chat = targets.save_dir.join("plugins/data").join(ID);
        std::fs::create_dir_all(chat.join("plugin")).unwrap();
        std::fs::write(chat.join("plugin/state"), "plugin").unwrap();
        let link = match place {
            "plugins" => targets.save_dir.join("plugins"),
            "data" => targets.save_dir.join("plugins/data"),
            "chat" => chat.clone(),
            "inside" => chat.join("plugin/link"),
            _ => unreachable!(),
        };
        if link.exists() {
            std::fs::rename(&link, dir.join("original")).unwrap();
        }
        symlink(&outside, &link).unwrap();
        finish(&targets, ID).unwrap();
        assert!(!targets.save_dir.join("unified").join(ID).exists());
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "outside",
            "{place}"
        );
        assert!(std::fs::symlink_metadata(&link).is_ok(), "{place}");
        clean(&dir);
    }
}
