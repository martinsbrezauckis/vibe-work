//! Where each agent keeps its setup, and how each file becomes plan rows.
//!
//! The `~/.claude/` agent: (`CLAUDE.md`, `skills/`, `agents/`, `commands/`, `settings.json`); its own MCP servers
//! live in `~/.claude.json`, an account file this scan never opens (one row says so); a project's `CLAUDE.md`,
//! `.mcp.json` and `.claude/skills/`. Codex: `$CODEX_HOME` or `~/.codex/` (`AGENTS.md`, `config.toml`, `skills/`,
//! `prompts/`; never `auth.json`). OpenCode: `<config>/opencode/` (`AGENTS.md`, `opencode.json(c)`, `skill(s)/`,
//! `agent(s)/`) and a project's `opencode.json(c)`. Cursor: `~/.cursor/mcp.json` and a project's `.cursor/mcp.json`
//! and `.cursor/rules/`. These are the same on Windows, macOS and Linux: both tools' documentation names only
//! `~/.config/opencode/` and `~/.cursor/`, with `~` the user's home folder (`%USERPROFILE%` on Windows).
//!
//! Depends on: `fs`, `mcp`. Called by: the crate root (`scan_with_payloads`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fs::Fs;
use crate::mcp::{self, Server};
use crate::{Agent, Found, Homes, Kind, Payload, Status, fingerprint};

/// The agent configured in `~/.claude/` (its display name is spelled once, in [`Agent::label`]).
const CLAUDE_CODE: Agent = Agent::ClaudeCode;

/// The most files one skill folder may hold, and their total size.
pub(crate) const SKILL_FILES: usize = 200;
const SKILL_BYTES: usize = 10 * 1024 * 1024;

struct Row<'a> {
    agent: Agent,
    kind: Kind,
    name: &'a str,
    source: &'a Path,
    bytes: &'a [u8],
}

fn found(row: &Row<'_>, status: Status, target: Option<&str>, note: Option<String>) -> Found {
    let source = row.source.display().to_string();
    Found {
        id: format!("{:?}:{:?}:{}:{source}", row.agent, row.kind, row.name),
        agent: row.agent,
        agent_label: row.agent.label().to_owned(),
        kind: row.kind,
        name: row.name.to_owned(),
        source,
        target: target.map(str::to_owned),
        status,
        note,
        detail: None,
        keys: Vec::new(),
        fingerprint: fingerprint(&[row.bytes]),
    }
}

fn cant(row: &Row<'_>, why: &str) -> (Found, Payload) {
    (
        found(row, Status::CantBring, None, Some(why.to_owned())),
        Payload::None,
    )
}

fn text(fs: &dyn Fs, path: &Path) -> Option<String> {
    fs.read(path)
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A personal instructions file: appended under a heading.
fn instructions(fs: &dyn Fs, agent: Agent, path: &Path, out: &mut Vec<(Found, Payload)>) {
    let Some(body) = text(fs, path) else {
        return;
    };
    if body.trim().is_empty() {
        return;
    }
    let row = Row {
        agent,
        kind: Kind::Instructions,
        name: "Personal instructions",
        source: path,
        bytes: body.as_bytes(),
    };
    out.push((
        found(
            &row,
            Status::Changed,
            Some("Your personal instructions"),
            Some(format!(
                "Added at the end under the heading \"Brought from {}\".",
                agent.label()
            )),
        ),
        Payload::Instructions(body),
    ));
}

/// A project's own instructions file: Vibe reads the project's `AGENTS.md`, nothing is copied.
fn project_instructions(fs: &dyn Fs, agent: Agent, path: &Path, out: &mut Vec<(Found, Payload)>) {
    if let Some(body) = text(fs, path) {
        let row = Row {
            agent,
            kind: Kind::Instructions,
            name: &file_name(path),
            source: path,
            bytes: body.as_bytes(),
        };
        out.push(cant(
            &row,
            "Project files stay in the project. Vibe reads the project's AGENTS.md: copy this file's text there if you want it.",
        ));
    }
}

/// Every `<root>/<name>/SKILL.md` folder: copied whole into Vibe Work's skills folder.
fn skills(fs: &dyn Fs, agent: Agent, root: &Path, out: &mut Vec<(Found, Payload)>) {
    for folder in fs.list(root).into_iter().filter(|e| e.dir) {
        let name = file_name(&folder.path);
        let manifest = folder.path.join("SKILL.md");
        let Some(head) = fs.read(&manifest) else {
            continue;
        };
        let mut files = Vec::new();
        let mut total = 0usize;
        let mut pending = vec![folder.path.clone()];
        let mut too_big = false;
        // The walk stops at the first file it cannot take or at a limit, and keeps nothing it read.
        'walk: while let Some(dir) = pending.pop() {
            for entry in fs.list(&dir) {
                if entry.dir {
                    pending.push(entry.path);
                    if pending.len() > SKILL_FILES {
                        too_big = true;
                        break 'walk;
                    }
                } else if entry.file {
                    let Some(bytes) = fs.read(&entry.path) else {
                        too_big = true;
                        break 'walk;
                    };
                    total += bytes.len();
                    let relative = entry
                        .path
                        .strip_prefix(&folder.path)
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    files.push((relative, bytes));
                    if files.len() > SKILL_FILES || total > SKILL_BYTES {
                        too_big = true;
                        break 'walk;
                    }
                }
            }
        }
        if too_big {
            files.clear();
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let all: Vec<&[u8]> = files.iter().map(|(_, b)| b.as_slice()).collect();
        let row = Row {
            agent,
            kind: Kind::Skill,
            name: &name,
            source: &folder.path,
            bytes: &head,
        };
        if too_big {
            out.push(cant(
                &row,
                "The folder holds a file that cannot be read here (a link, a key file or a file over 1 MB), or too many files.",
            ));
            continue;
        }
        let mut row_found = found(&row, Status::Ready, Some("Vibe Work's skills"), None);
        row_found.fingerprint = fingerprint(&all);
        out.push((row_found, Payload::Skill(files)));
    }
}

/// Markdown definitions Vibe Work does not bring (subagents, commands, rules), one row each.
fn listed(
    fs: &dyn Fs,
    agent: Agent,
    kind: Kind,
    root: &Path,
    why: &str,
    out: &mut Vec<(Found, Payload)>,
) {
    for entry in fs.list(root).into_iter().filter(|e| e.file) {
        let name = file_name(&entry.path);
        let row = Row {
            agent,
            kind,
            name: &name,
            source: &entry.path,
            bytes: name.as_bytes(),
        };
        out.push(cant(&row, why));
    }
}

/// One MCP server row.
/// The longest command line or address a row may have (the confirmation shows it whole).
const MAX_DETAIL: usize = 400;

fn server(
    agent: Agent,
    name: &str,
    source: &Path,
    server: Server,
    bytes: &[u8],
    from_project: bool,
) -> (Found, Payload) {
    let row = Row {
        agent,
        kind: Kind::McpServer,
        name,
        source,
        bytes,
    };
    let keys = server.keys();
    let detail = server.detail();
    let inline = server.inline_keys();
    let risky = server.risky_env();
    let refused = if !inline.is_empty() {
        Some(format!(
            "A key is written into {}; add it in Tools so the key stays in your vault.",
            inline.join(", ")
        ))
    } else if !risky.is_empty() {
        Some(format!(
            "It sets {}, which changes how programs run; add it in Tools after review.",
            risky.join(", ")
        ))
    } else if from_project && matches!(server, Server::Stdio { .. }) {
        Some(
            "A program from a project's own files is not brought; add it in Tools after review."
                .to_owned(),
        )
    } else if detail.chars().count() > MAX_DETAIL {
        Some("Its command line is too long to show in full; add it in Tools.".to_owned())
    } else {
        None
    };
    let (status, target, note) = match &server {
        _ if refused.is_some() => (Status::CantBring, None, refused),
        Server::Http { url, .. } if !url.starts_with("https://") => (
            Status::CantBring,
            None,
            Some("Only https addresses can be brought.".to_owned()),
        ),
        Server::Http { .. } if keys.len() > 1 => (
            Status::CantBring,
            None,
            Some("It sends more than one key; add it in Tools.".to_owned()),
        ),
        // The row's keys already tell the Settings row to say where the key goes (vocabulary `importKeyToVault`).
        Server::Http { .. } if keys.len() == 1 => {
            (Status::Changed, Some("Vibe Work's MCP servers"), None)
        }
        Server::Stdio { .. } if !keys.is_empty() => (
            Status::CantBring,
            None,
            Some("A program server with a key in its settings: add it in Tools so the key stays in your vault.".to_owned()),
        ),
        _ => (Status::Ready, Some("Vibe Work's MCP servers"), None),
    };
    let mut row_found = found(&row, status, target, note);
    row_found.detail = Some(detail);
    row_found.keys = keys;
    row_found.fingerprint = fingerprint(&[bytes, name.as_bytes()]);
    (row_found, Payload::Server(server))
}

fn json_servers(
    fs: &dyn Fs,
    agent: Agent,
    path: &Path,
    key: &str,
    from_project: bool,
    out: &mut Vec<(Found, Payload)>,
) {
    let Some(raw) = fs.read(path) else {
        return;
    };
    let Some(value) = std::str::from_utf8(&raw).ok().and_then(mcp::jsonc) else {
        return;
    };
    for (name, entry) in value
        .get(key)
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let parsed = if agent == Agent::OpenCode {
            mcp::from_opencode(entry)
        } else {
            mcp::from_claude(entry)
        };
        if let Some(parsed) = parsed {
            out.push(server(agent, name, path, parsed, &raw, from_project));
        }
    }
}

fn claude(fs: &dyn Fs, homes: &Homes, out: &mut Vec<(Found, Payload)>) {
    let root = homes.home.join(".claude");
    instructions(fs, CLAUDE_CODE, &root.join("CLAUDE.md"), out);
    skills(fs, CLAUDE_CODE, &root.join("skills"), out);
    listed(
        fs,
        CLAUDE_CODE,
        Kind::Subagent,
        &root.join("agents"),
        "Vibe Work does not bring subagents yet; add them in Vibe's agents folder yourself.",
        out,
    );
    listed(
        fs,
        CLAUDE_CODE,
        Kind::Command,
        &root.join("commands"),
        "Vibe Work does not bring custom commands; a skill can do the same.",
        out,
    );
    settings_rows(fs, CLAUDE_CODE, &root.join("settings.json"), out);
    let account = homes.home.join(".claude.json");
    if fs.exists(&account) {
        let row = Row {
            agent: CLAUDE_CODE,
            kind: Kind::McpServer,
            name: &format!("{}'s own MCP servers", CLAUDE_CODE.label()),
            source: &account,
            bytes: b"",
        };
        out.push(cant(
            &row,
            &format!(
                "{} keeps these in its account file, which Vibe Work does not open. Add them in Tools.",
                CLAUDE_CODE.label()
            ),
        ));
    }
    if let Some(project) = &homes.project {
        project_instructions(fs, CLAUDE_CODE, &project.join("CLAUDE.md"), out);
        json_servers(
            fs,
            CLAUDE_CODE,
            &project.join(".mcp.json"),
            "mcpServers",
            true,
            out,
        );
        skills(
            fs,
            CLAUDE_CODE,
            &project.join(".claude").join("skills"),
            out,
        );
    }
}

/// Permission and hook settings in a JSON settings file: listed, not brought.
fn settings_rows(fs: &dyn Fs, agent: Agent, path: &Path, out: &mut Vec<(Found, Payload)>) {
    let Some(raw) = fs.read(path) else {
        return;
    };
    let Some(value) = std::str::from_utf8(&raw).ok().and_then(mcp::jsonc) else {
        return;
    };
    for (key, kind, why) in [
        (
            "permissions",
            Kind::Permissions,
            "Vibe Work uses its own access modes; pick one in the chat.",
        ),
        (
            "permission",
            Kind::Permissions,
            "Vibe Work uses its own access modes; pick one in the chat.",
        ),
        (
            "hooks",
            Kind::Hooks,
            "Hooks run programs; add them yourself after review.",
        ),
        (
            "agent",
            Kind::Subagent,
            "Vibe Work does not bring subagents yet.",
        ),
    ] {
        if value.get(key).is_some() {
            let row = Row {
                agent,
                kind,
                name: key,
                source: path,
                bytes: &raw,
            };
            out.push(cant(&row, why));
        }
    }
}

fn codex(fs: &dyn Fs, homes: &Homes, out: &mut Vec<(Found, Payload)>) {
    let root = homes
        .codex_home
        .clone()
        .unwrap_or_else(|| homes.home.join(".codex"));
    instructions(fs, Agent::Codex, &root.join("AGENTS.md"), out);
    skills(fs, Agent::Codex, &root.join("skills"), out);
    listed(
        fs,
        Agent::Codex,
        Kind::Command,
        &root.join("prompts"),
        "Vibe Work does not bring custom prompts; a skill can do the same.",
        out,
    );
    let config = root.join("config.toml");
    let Some(raw) = fs.read(&config) else {
        return;
    };
    let Some(table) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
    else {
        return;
    };
    for (name, entry) in table
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
    {
        if let Some(parsed) = mcp::from_codex(entry) {
            out.push(server(Agent::Codex, name, &config, parsed, &raw, false));
        }
    }
    for key in ["approval_policy", "sandbox_mode"] {
        if table.contains_key(key) {
            let row = Row {
                agent: Agent::Codex,
                kind: Kind::Permissions,
                name: key,
                source: &config,
                bytes: &raw,
            };
            out.push(cant(
                &row,
                "Vibe Work uses its own access modes; pick one in the chat.",
            ));
        }
    }
}

fn opencode(fs: &dyn Fs, homes: &Homes, out: &mut Vec<(Found, Payload)>) {
    let root = homes.config_dir.join("opencode");
    instructions(fs, Agent::OpenCode, &root.join("AGENTS.md"), out);
    for folder in ["skill", "skills"] {
        skills(fs, Agent::OpenCode, &root.join(folder), out);
    }
    for folder in ["agent", "agents"] {
        listed(
            fs,
            Agent::OpenCode,
            Kind::Subagent,
            &root.join(folder),
            "Vibe Work does not bring subagents yet; add them in Vibe's agents folder yourself.",
            out,
        );
    }
    let mut configs: Vec<PathBuf> = vec![root.join("opencode.json"), root.join("opencode.jsonc")];
    if let Some(project) = &homes.project {
        configs.extend([
            project.join("opencode.json"),
            project.join("opencode.jsonc"),
        ]);
    }
    for config in configs {
        let from_project = homes
            .project
            .as_ref()
            .is_some_and(|p| config.starts_with(p));
        json_servers(fs, Agent::OpenCode, &config, "mcp", from_project, out);
        settings_rows(fs, Agent::OpenCode, &config, out);
    }
}

fn cursor(fs: &dyn Fs, homes: &Homes, out: &mut Vec<(Found, Payload)>) {
    json_servers(
        fs,
        Agent::Cursor,
        &homes.home.join(".cursor").join("mcp.json"),
        "mcpServers",
        false,
        out,
    );
    if let Some(project) = &homes.project {
        json_servers(
            fs,
            Agent::Cursor,
            &project.join(".cursor").join("mcp.json"),
            "mcpServers",
            true,
            out,
        );
        listed(
            fs,
            Agent::Cursor,
            Kind::Rules,
            &project.join(".cursor").join("rules"),
            "Cursor rules stay in the project. Vibe reads the project's AGENTS.md.",
            out,
        );
    }
}

/// The agents whose personal setup holds something that can be brought (instructions, skills or a connector
/// config), in the order of [`Agent`]. Existence checks only: no file is opened or listed. Project files and the
/// `~/.claude/` agent's account file are not looked at.
pub(crate) fn detect(fs: &dyn Fs, homes: &Homes) -> Vec<Agent> {
    let claude = homes.home.join(".claude");
    let codex = homes
        .codex_home
        .clone()
        .unwrap_or_else(|| homes.home.join(".codex"));
    let opencode = homes.config_dir.join("opencode");
    let marks: [(Agent, Vec<PathBuf>); 4] = [
        (
            CLAUDE_CODE,
            vec![claude.join("CLAUDE.md"), claude.join("skills")],
        ),
        (
            Agent::Codex,
            vec![
                codex.join("AGENTS.md"),
                codex.join("config.toml"),
                codex.join("skills"),
            ],
        ),
        (
            Agent::OpenCode,
            [
                "AGENTS.md",
                "opencode.json",
                "opencode.jsonc",
                "skill",
                "skills",
            ]
            .iter()
            .map(|name| opencode.join(name))
            .collect(),
        ),
        (
            Agent::Cursor,
            vec![homes.home.join(".cursor").join("mcp.json")],
        ),
    ];
    marks
        .into_iter()
        .filter(|(_, paths)| paths.iter().any(|path| fs.exists(path)))
        .map(|(agent, _)| agent)
        .collect()
}

/// Every agent's rows, in the order of [`Agent`].
pub(crate) fn scan_all(fs: &dyn Fs, homes: &Homes) -> Vec<(Found, Payload)> {
    let mut out = Vec::new();
    claude(fs, homes, &mut out);
    codex(fs, homes, &mut out);
    opencode(fs, homes, &mut out);
    cursor(fs, homes, &mut out);
    out
}
