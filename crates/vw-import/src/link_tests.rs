//! Import paths with linked ancestors must be refused and visible in the plan.
//!
//! PREDICT: the current reader checks only the final file, so a project `.opencode` directory
//! link and a linked parent of a config file both expose the outside file to the scan. The
//! resulting rows contain the outside server instead of a linked-path refusal.

use std::path::Path;

use super::fs::{Disk, Fs};
use super::{Agent, Homes, Kind, Status, Taken, scan};
#[cfg(unix)]
use super::{Payload, scan_with_payloads};

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn homes(root: &Path, project: &Path) -> Homes {
    Homes {
        home: root.join("empty-home"),
        codex_home: None,
        config_dir: root.join("empty-config"),
        project: Some(project.to_path_buf()),
    }
}

fn assert_linked_row(rows: &[super::Found], agent: Agent, path: &Path) {
    let source = path.display().to_string();
    let linked: Vec<_> = rows.iter().filter(|row| row.source == source).collect();
    assert_eq!(
        linked.len(),
        1,
        "expected one refusal for {source}: {rows:#?}"
    );
    let row = linked[0];
    assert_eq!(row.agent, agent);
    assert_eq!(row.kind, Kind::McpServer);
    assert_eq!(row.status, Status::CantBring);
    assert_eq!(row.note.as_deref(), vw_vocabulary::text("link"));
    assert!(row.target.is_none() && row.keys.is_empty());
}

#[cfg(unix)]
#[test]
fn project_opencode_directory_link_is_reported_without_importing_outside_server() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let project = root_path.join("project");
    let outside = root_path.join("outside");
    std::fs::create_dir(&project).unwrap();
    write(
        &outside.join("opencode.json"),
        r#"{"mcp":{"private":{"type":"remote","url":"https://example.test/mcp","headers":{"Authorization":"dummy"}}}}"#,
    );
    symlink(&outside, project.join(".opencode")).unwrap();
    let config = project.join(".opencode/opencode.json");
    let setup = homes(&root_path, &project);
    let rows = scan(&Disk, &setup, &Taken::default());
    assert!(rows.iter().all(|row| row.name != "private"), "{rows:#?}");
    assert_linked_row(&rows, Agent::OpenCode, &config);
    let rescanned = scan_with_payloads(&Disk, &setup, &Taken::default());
    assert!(rescanned.iter().all(|(row, _)| row.name != "private"));
    assert!(rescanned.iter().any(|(row, payload)| {
        row.source == config.display().to_string()
            && row.status == Status::CantBring
            && matches!(payload, Payload::None)
    }));
    let parallel = std::thread::scope(|scope| {
        let first = scope.spawn(|| scan(&Disk, &setup, &Taken::default()));
        let second = scope.spawn(|| scan(&Disk, &setup, &Taken::default()));
        [first.join().unwrap(), second.join().unwrap()]
    });
    for rows in parallel {
        assert!(rows.iter().all(|row| row.name != "private"));
        assert_linked_row(&rows, Agent::OpenCode, &config);
    }
    assert!(Disk.read(&config).is_none());
    assert!(Disk.list(&project.join(".opencode")).is_empty());
    assert!(Disk.read(&project.join("missing.json")).is_none());
    assert!(Disk.list(&project.join("missing-folder")).is_empty());
}

#[cfg(unix)]
#[test]
fn linked_parent_and_linked_file_are_refused_and_reported() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let project = root_path.join("project");
    let outside = root_path.join("outside");
    std::fs::create_dir(&project).unwrap();
    write(
        &outside.join("mcp.json"),
        r#"{"mcpServers":{"private":{"url":"https://example.test/mcp"}}}"#,
    );
    write(&outside.join("SKILL.md"), "---\nname: outside\n---\n");
    symlink(&outside, project.join(".cursor")).unwrap();
    symlink(outside.join("mcp.json"), project.join(".mcp.json")).unwrap();
    std::fs::create_dir_all(project.join(".claude/skills")).unwrap();
    symlink(&outside, project.join(".claude/skills/linked-skill")).unwrap();
    let parent_config = project.join(".cursor/mcp.json");
    let leaf_config = project.join(".mcp.json");
    let rows = scan(&Disk, &homes(&root_path, &project), &Taken::default());
    assert!(rows.iter().all(|row| row.name != "private"), "{rows:#?}");
    assert_linked_row(&rows, Agent::Cursor, &parent_config);
    assert_linked_row(&rows, Agent::ClaudeCode, &leaf_config);
    let skill = rows
        .iter()
        .find(|row| {
            row.source
                == project
                    .join(".claude/skills/linked-skill")
                    .display()
                    .to_string()
        })
        .unwrap();
    assert_eq!((skill.kind, skill.status), (Kind::Skill, Status::CantBring));
    assert_eq!(skill.note.as_deref(), vw_vocabulary::text("link"));
    assert!(Disk.read(&parent_config).is_none());
    assert!(Disk.read(&leaf_config).is_none());
}

#[cfg(windows)]
#[test]
fn project_opencode_junction_is_reported_without_importing_outside_server() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let project = root_path.join("project");
    let outside = root_path.join("outside");
    std::fs::create_dir(&project).unwrap();
    write(
        &outside.join("opencode.json"),
        r#"{"mcp":{"private":{"type":"remote","url":"https://example.test/mcp"}}}"#,
    );
    let result = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(project.join(".opencode"))
        .arg(&outside)
        .status()
        .unwrap();
    assert!(result.success());
    let config = project.join(".opencode/opencode.json");
    let rows = scan(&Disk, &homes(&root_path, &project), &Taken::default());
    assert!(rows.iter().all(|row| row.name != "private"), "{rows:#?}");
    assert_linked_row(&rows, Agent::OpenCode, &config);
    assert!(Disk.read(&config).is_none());
}
