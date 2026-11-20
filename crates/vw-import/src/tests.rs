//! Fixture homes for the four agents (fake values only).
//!
//! PREDICT: before this crate existed nothing here compiled. The plan must list instructions, skills and servers
//! with their status, name a key without its value, and never ask for a sign-in or secret file (a recording reader
//! sees every path the scan asks for). A canonical fixture root keeps OS temp-directory aliases out of these paths.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::fs::{Disk, Entry, Fs, never_read};
use super::{Agent, Homes, Kind, Status, Taken, scan, scan_with_payloads};

const CLAUDE_CODE: Agent = Agent::ClaudeCode;

const CANARY: &str = "fake-canary-value-7f3a";
const KEY: &str = "fake-github-token-for-tests-only";

/// Records every path the scan asks for, then reads through the real reader.
struct Recording {
    asked: RefCell<Vec<PathBuf>>,
}

impl Fs for Recording {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        self.asked.borrow_mut().push(path.to_path_buf());
        Disk.read(path)
    }
    fn list(&self, path: &Path) -> Vec<Entry> {
        Disk.list(path)
    }
    fn exists(&self, path: &Path) -> bool {
        Disk.exists(path)
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fixture() -> (tempfile::TempDir, Homes) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let home = root.join("home");
    let project = root.join("project");
    // The `~/.claude/` agent.
    write(&home.join(".claude/CLAUDE.md"), "Prefer small commits.\n");
    write(
        &home.join(".claude/skills/review/SKILL.md"),
        "---\nname: review\n---\nReview.\n",
    );
    write(&home.join(".claude/skills/review/notes.md"), "notes\n");
    write(
        &home.join(".claude/skills/deploy/SKILL.md"),
        "---\nname: deploy\n---\nDeploy.\n",
    );
    write(&home.join(".claude/agents/planner.md"), "planner\n");
    write(
        &home.join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(ls)"]},"hooks":{}}"#,
    );
    write(&home.join(".claude/.credentials.json"), CANARY);
    write(
        &home.join(".claude.json"),
        &format!(r#"{{"oauthAccount":"{CANARY}","mcpServers":{{}}}}"#),
    );
    write(&project.join("CLAUDE.md"), "Project rules.\n");
    write(&project.join(".env"), &format!("TOKEN={CANARY}\n"));
    write(
        &project.join(".mcp.json"),
        &format!(
            r#"{{"mcpServers":{{
              "github":{{"command":"npx","args":["-y","@modelcontextprotocol/server-github"],"env":{{"GITHUB_TOKEN":"{KEY}"}}}},
              "docs":{{"type":"http","url":"https://docs.example.com/mcp","headers":{{"Authorization":"Bearer {KEY}"}}}},
              "files":{{"command":"node","args":["server.js","--root","my folder"]}}
            }}}}"#
        ),
    );
    // Codex.
    write(&home.join(".codex/AGENTS.md"), "Answer briefly.\n");
    write(&home.join(".codex/auth.json"), CANARY);
    write(
        &home.join(".codex/config.toml"),
        "approval_policy = \"on-request\"\n[mcp_servers.search]\ncommand = \"uvx\"\nargs = [\"search-mcp\"]\n[mcp_servers.remote]\nurl = \"https://remote.example.com/mcp\"\nenv = { API_KEY = \"$API_KEY\" }\n",
    );
    // OpenCode (JSON with comments).
    let config = root.join("config");
    write(
        &config.join("opencode/opencode.jsonc"),
        &format!(
            "{{\n // my servers\n \"mcp\": {{\"notes\": {{\"type\": \"remote\", \"url\": \"https://notes.example.com/mcp\", \"headers\": {{\"X-Api-Key\": \"{KEY}\"}}}},\n /* local */ \"shell\": {{\"type\": \"local\", \"command\": [\"bun\", \"x\", \"shell-mcp\"]}}}},\n \"permission\": {{\"edit\": \"ask\"}}\n}}\n"
        ),
    );
    // OpenCode's documented global agents folder (opencode.ai/docs/agents) and its older singular name.
    write(&config.join("opencode/agents/reviewer.md"), "reviewer\n");
    write(&config.join("opencode/agent/tester.md"), "tester\n");
    write(&home.join(".local/share/opencode/auth.json"), CANARY);
    // Cursor.
    write(
        &home.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"web":{"url":"http://localhost:9000/mcp"}}}"#,
    );
    write(&project.join(".cursor/rules/style.mdc"), "style\n");
    let homes = Homes {
        home,
        codex_home: None,
        config_dir: config,
        project: Some(project),
    };
    (dir, homes)
}

fn row<'a>(rows: &'a [super::Found], agent: Agent, kind: Kind, name: &str) -> &'a super::Found {
    rows.iter()
        .find(|r| r.agent == agent && r.kind == kind && r.name == name)
        .unwrap_or_else(|| panic!("no {agent:?} {kind:?} {name}: {rows:#?}"))
}

#[test]
fn a_claude_code_home_becomes_plan_rows_without_any_key_value() {
    let (_dir, homes) = fixture();
    let rows = scan(&Disk, &homes, &Taken::default());
    assert_eq!(
        row(
            &rows,
            CLAUDE_CODE,
            Kind::Instructions,
            "Personal instructions"
        )
        .status,
        Status::Changed
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Skill, "review").status,
        Status::Ready
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Skill, "deploy").status,
        Status::Ready
    );
    let github = row(&rows, CLAUDE_CODE, Kind::McpServer, "github");
    assert_eq!(github.status, Status::CantBring);
    assert_eq!(github.keys, ["GITHUB_TOKEN"]);
    let docs = row(&rows, CLAUDE_CODE, Kind::McpServer, "docs");
    assert_eq!(docs.status, Status::Changed, "{docs:?}");
    assert_eq!(docs.keys, ["Authorization"]);
    let files = row(&rows, CLAUDE_CODE, Kind::McpServer, "files");
    assert_eq!(
        files.detail.as_deref(),
        Some("node server.js --root \"my folder\"")
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Permissions, "permissions").status,
        Status::CantBring
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Hooks, "hooks").status,
        Status::CantBring
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Subagent, "planner.md").status,
        Status::CantBring
    );
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Instructions, "CLAUDE.md").status,
        Status::CantBring
    );
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(
        !shown.contains(KEY) && !shown.contains(CANARY),
        "a value reached the plan"
    );
    let payloads = format!("{:?}", scan_with_payloads(&Disk, &homes, &Taken::default()));
    assert!(
        !payloads.contains(KEY) && !payloads.contains(CANARY),
        "a value reached Debug"
    );
}

#[test]
fn sign_in_and_secret_files_are_never_asked_for() {
    let (_dir, homes) = fixture();
    let recording = Recording {
        asked: RefCell::new(Vec::new()),
    };
    let rows = scan(&recording, &homes, &Taken::default());
    let asked = recording.asked.borrow();
    assert!(!asked.is_empty());
    for path in asked.iter() {
        assert!(!never_read(path), "asked for {}", path.display());
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name != "auth.json" && !name.starts_with(".env") && name != ".claude.json",
            "{name}"
        );
    }
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(!shown.contains(CANARY));
    // The account file is only named, never read.
    assert_eq!(
        row(
            &rows,
            CLAUDE_CODE,
            Kind::McpServer,
            &format!("{}'s own MCP servers", CLAUDE_CODE.label())
        )
        .status,
        Status::CantBring
    );
    // The real reader refuses them even when asked directly.
    for file in [
        ".env",
        ".codex/auth.json",
        ".claude/.credentials.json",
        ".claude.json",
    ] {
        let path = if file == ".env" {
            homes.project.as_ref().unwrap().join(file)
        } else {
            homes.home.join(file)
        };
        assert!(path.exists() && Disk.read(&path).is_none(), "{file}");
    }
}

#[test]
fn codex_opencode_and_cursor_formats_are_read() {
    let (_dir, homes) = fixture();
    let rows = scan(&Disk, &homes, &Taken::default());
    assert_eq!(
        row(
            &rows,
            Agent::Codex,
            Kind::Instructions,
            "Personal instructions"
        )
        .status,
        Status::Changed
    );
    let search = row(&rows, Agent::Codex, Kind::McpServer, "search");
    assert_eq!(
        (search.status, search.detail.as_deref()),
        (Status::Ready, Some("uvx search-mcp"))
    );
    let remote = row(&rows, Agent::Codex, Kind::McpServer, "remote");
    assert_eq!(
        remote.status,
        Status::Ready,
        "an env reference is not a key: {remote:?}"
    );
    assert_eq!(
        row(&rows, Agent::Codex, Kind::Permissions, "approval_policy").status,
        Status::CantBring
    );
    let notes = row(&rows, Agent::OpenCode, Kind::McpServer, "notes");
    assert_eq!(
        (notes.status, notes.keys.clone()),
        (Status::Changed, vec!["X-Api-Key".to_owned()])
    );
    assert_eq!(
        row(&rows, Agent::OpenCode, Kind::McpServer, "shell")
            .detail
            .as_deref(),
        Some("bun x shell-mcp")
    );
    assert_eq!(
        row(&rows, Agent::OpenCode, Kind::Permissions, "permission").status,
        Status::CantBring
    );
    // PREDICT: before OpenCode's agents folders were read, neither subagent had a row (the person saw nothing).
    for name in ["reviewer.md", "tester.md"] {
        assert_eq!(
            row(&rows, Agent::OpenCode, Kind::Subagent, name).status,
            Status::CantBring
        );
    }
    let web = row(&rows, Agent::Cursor, Kind::McpServer, "web");
    assert_eq!(web.status, Status::CantBring, "not https");
    assert_eq!(
        row(&rows, Agent::Cursor, Kind::Rules, "style.mdc").status,
        Status::CantBring
    );
}

/// PREDICT: a project that stores OpenCode's MCP setup in `.opencode/opencode.json(c)` must show
/// both its remote servers and its locally launched server in the plan; the local program stays
/// unavailable for import under the existing project-program rule.
#[test]
fn opencode_project_dot_directory_config_is_in_the_plan() {
    let (_dir, homes) = fixture();
    let project = homes.project.as_ref().unwrap();
    write(
        &project.join(".opencode/opencode.jsonc"),
        r#"{"mcp":{"project-docs":{"type":"remote","url":"https://docs.example.test/mcp"},"project-runner":{"type":"local","command":["node","server.js"]}}}"#,
    );
    write(
        &project.join(".opencode/opencode.json"),
        r#"{"mcp":{"project-wiki":{"type":"remote","url":"https://wiki.example.test/mcp"}}}"#,
    );
    let rows = scan(&Disk, &homes, &Taken::default());
    let docs = row(&rows, Agent::OpenCode, Kind::McpServer, "project-docs");
    assert_eq!(docs.status, Status::Ready, "{docs:?}");
    let wiki = row(&rows, Agent::OpenCode, Kind::McpServer, "project-wiki");
    assert_eq!(wiki.status, Status::Ready, "{wiki:?}");
    let runner = row(&rows, Agent::OpenCode, Kind::McpServer, "project-runner");
    assert_eq!(runner.status, Status::CantBring, "{runner:?}");
}

#[test]
fn names_vibe_work_already_uses_cannot_be_brought_and_changes_show() {
    let (_dir, homes) = fixture();
    let taken = Taken {
        skills: vec!["review".to_owned()],
        servers: vec!["search".to_owned()],
    };
    let rows = scan(&Disk, &homes, &taken);
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Skill, "review").status,
        Status::CantBring
    );
    assert_eq!(
        row(&rows, Agent::Codex, Kind::McpServer, "search").status,
        Status::CantBring
    );
    let before = row(&rows, CLAUDE_CODE, Kind::Skill, "deploy")
        .fingerprint
        .clone();
    write(
        &homes.home.join(".claude/skills/deploy/SKILL.md"),
        "changed\n",
    );
    let after = scan(&Disk, &homes, &taken);
    assert_ne!(
        row(&after, CLAUDE_CODE, Kind::Skill, "deploy").fingerprint,
        before
    );
}

#[test]
fn a_skill_over_the_limits_is_not_brought_and_its_walk_stops() {
    let (_dir, homes) = fixture();
    let big = homes.home.join(".claude/skills/huge");
    write(&big.join("SKILL.md"), "---\nname: huge\n---\n");
    for i in 0..(super::agents::SKILL_FILES + 5) {
        write(&big.join(format!("f{i:04}.md")), "x\n");
    }
    let recording = Recording {
        asked: RefCell::new(Vec::new()),
    };
    let rows = scan(&recording, &homes, &Taken::default());
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::Skill, "huge").status,
        Status::CantBring
    );
    let read_in_huge = recording
        .asked
        .borrow()
        .iter()
        .filter(|p| p.starts_with(&big))
        .count();
    assert!(
        read_in_huge <= super::agents::SKILL_FILES + 2,
        "{read_in_huge}"
    );
}

// PREDICT: before keys in arguments and addresses counted as keys, `inline` was Ready with the value in its
// command line, `query` showed its token, and the project's program server and `preload` were Ready.
#[test]
fn inline_keys_risky_environment_and_project_programs_are_not_brought() {
    let (_dir, homes) = fixture();
    write(
        &homes.home.join(".cursor/mcp.json"),
        &format!(
            r#"{{"mcpServers":{{
              "inline":{{"command":"tool","args":["serve","--api-key","{KEY}","TOKEN={KEY}"]}},
              "query":{{"url":"https://u:{KEY}@api.example.com/mcp?token={KEY}&region=eu"}},
              "preload":{{"command":"tool","env":{{"LD_PRELOAD":"/tmp/x.so","MODE":"fast"}}}},
              "fragment":{{"url":"https://api.example.com/mcp#access_token={KEY}"}},
              "plain":{{"command":"tool","args":["serve"],"env":{{"MODE":"fast"}}}}
            }}}}"#
        ),
    );
    let rows = scan(&Disk, &homes, &Taken::default());
    for name in ["inline", "query", "preload", "fragment"] {
        assert_eq!(
            row(&rows, Agent::Cursor, Kind::McpServer, name).status,
            Status::CantBring,
            "{name}"
        );
    }
    let plain = row(&rows, Agent::Cursor, Kind::McpServer, "plain");
    // A written-out environment value counts as a key whatever its name: not brought, and not shown.
    assert_eq!(plain.status, Status::CantBring);
    assert_eq!(
        plain.detail.as_deref(),
        Some("MODE=\u{2022}\u{2022}\u{2022} tool serve")
    );
    // A program from the project's own files is not brought.
    assert_eq!(
        row(&rows, CLAUDE_CODE, Kind::McpServer, "files").status,
        Status::CantBring
    );
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(!shown.contains(KEY), "a value reached the plan: {shown}");
    assert!(
        row(&rows, Agent::Cursor, Kind::McpServer, "query")
            .detail
            .as_deref()
            .is_some_and(|d| d == "https://api.example.com/mcp?\u{2022}\u{2022}\u{2022}")
    );
}

/// PREDICT: before `detect` existed this did not compile. With it: each agent whose personal setup has something
/// that can be brought is named, in the order of `Agent`, by existence checks only (no file is read); an empty
/// home names none; a `~/.claude/` folder holding only settings is not a setup to offer.
#[test]
fn detect_names_agents_with_a_setup_without_reading_any_file() {
    struct Recording<'a>(&'a Disk, std::cell::RefCell<Vec<std::path::PathBuf>>);
    impl super::fs::Fs for Recording<'_> {
        fn read(&self, path: &Path) -> Option<Vec<u8>> {
            self.1.borrow_mut().push(path.to_path_buf());
            self.0.read(path)
        }
        fn list(&self, path: &Path) -> Vec<super::fs::Entry> {
            self.1.borrow_mut().push(path.to_path_buf());
            self.0.list(path)
        }
        fn exists(&self, path: &Path) -> bool {
            self.0.exists(path)
        }
    }
    let (_dir, homes) = fixture();
    let fs = Recording(&Disk, std::cell::RefCell::default());
    assert_eq!(
        super::detect(&fs, &homes),
        vec![
            Agent::ClaudeCode,
            Agent::Codex,
            Agent::OpenCode,
            Agent::Cursor
        ]
    );
    assert!(
        fs.1.borrow().is_empty(),
        "read or listed: {:?}",
        fs.1.borrow()
    );
    let empty = tempfile::tempdir().unwrap();
    let empty_root = empty.path().canonicalize().unwrap();
    let only_settings = Homes {
        home: empty_root.join("home"),
        codex_home: None,
        config_dir: empty_root.join("config"),
        project: None,
    };
    write(&only_settings.home.join(".claude/settings.json"), "{}");
    assert_eq!(super::detect(&Disk, &only_settings), Vec::<Agent>::new());
}
