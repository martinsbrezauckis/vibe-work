//! Command-started (`transport = "stdio"`) MCP servers in Vibe Work's own Vibe config: write, change, read, and the
//! `disabled` flag the pause rule uses. Built on the shared MCP row editor and the atomic editor of `mcp_keys_config`
//! (same file rules: only Vibe Work's own home, `toml_edit` keeps the rest of the file).
//!
//! Depends on: `mcp_rows`, `mcp_keys_config::edit_file`, `toml`, `toml_edit`. Called by: `command_servers`,
//! `command_pause`, `connectors_config` (the view).
//! Invariants:
//! - Only plain values are ever written (`command_secret` refuses the rest before this module is called).
//! - A change replaces `command`, `args`, `cwd` and `env` only; every other key of the entry (`disabled`, timeouts, ...)
//!   stays as it was.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use toml_edit::{Array, DocumentMut, InlineTable, Table, Value, value};

use crate::mcp_keys_config::{ConfigEditError, edit_file};
use crate::mcp_rows::{RowMut, edit_rows, read_rows};

/// What Vibe needs to start a command server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandEntry {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

fn fill(mut row: RowMut<'_>, entry: &CommandEntry) {
    row.set("transport", Value::from("stdio"));
    row.set("command", Value::from(entry.command.as_str()));
    let mut args = Array::new();
    for arg in &entry.args {
        args.push(arg.as_str());
    }
    row.set("args", Value::Array(args));
    match &entry.cwd {
        Some(cwd) => {
            row.set("cwd", Value::from(cwd.as_str()));
        }
        None => {
            row.remove("cwd");
        }
    }
    if entry.env.is_empty() {
        row.remove("env");
    } else {
        let mut env = InlineTable::new();
        for (k, v) in &entry.env {
            env.insert(k, v.as_str().into());
        }
        row.set("env", Value::InlineTable(env));
    }
}

/// Adds the command server `name`.
///
/// # Errors
/// [`ConfigEditError`] (`NameTaken` when a server of that name exists).
pub(crate) fn add_command_entry(
    home: &Path,
    name: &str,
    entry: &CommandEntry,
) -> Result<(), ConfigEditError> {
    edit_file(home, |doc| {
        let mut list = edit_rows(doc)?;
        if list.has_name(name)? {
            return Err(ConfigEditError::NameTaken);
        }
        let mut table = Table::new();
        table.insert("name", value(name));
        fill(RowMut::Table(&mut table), entry);
        list.push(table);
        Ok(true)
    })
    .map(|_| ())
}

/// Replaces the program, arguments, working folder and environment of the command server `name`.
///
/// # Errors
/// [`ConfigEditError`] (`NotACommandServer` when `name` is absent or not a command server).
pub(crate) fn replace_command_entry(
    home: &Path,
    name: &str,
    entry: &CommandEntry,
) -> Result<(), ConfigEditError> {
    edit_file(home, |doc| {
        let mut list = edit_rows(doc)?;
        let mut matches = 0;
        list.for_each(|row| {
            if row.get_str("name") == Some(name) && row.get_str("transport") == Some("stdio") {
                matches += 1;
            }
            Ok(false)
        })?;
        if matches != 1 {
            return Err(ConfigEditError::NotACommandServer);
        }
        list.for_each(|row| {
            if row.get_str("name") == Some(name) && row.get_str("transport") == Some("stdio") {
                fill(row, entry);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        Ok(true)
    })
    .map(|_| ())
}

fn parse_entry(table: &toml::Table) -> CommandEntry {
    let text = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    CommandEntry {
        command: text("command").unwrap_or_default(),
        args: table
            .get("args")
            .and_then(toml::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        cwd: text("cwd").filter(|c| !c.is_empty()),
        env: table
            .get("env")
            .and_then(toml::Value::as_table)
            .map(|env| {
                env.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The command entry of one parsed MCP row, `None` for another transport.
pub(crate) fn command_entry_of(table: &toml::Table) -> Option<CommandEntry> {
    (table.get("transport").and_then(toml::Value::as_str) == Some("stdio"))
        .then(|| parse_entry(table))
}

/// Vibe 3.1.4's `normalize_mcp_server_name`: the wire name and persisted name are both normalized
/// before Vibe matches them. A literal comparison here would miss a command target with an alias.
pub(crate) fn vibe_server_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    mapped
        .trim_matches(|character| character == '_' || character == '-')
        .chars()
        .take(256)
        .collect()
}

/// Reads one target from Vibe's config, refusing an unreadable or ambiguous config before a raw toggle.
pub(crate) fn command_entry_checked(
    home: &Path,
    name: &str,
) -> Result<Option<CommandEntry>, ConfigEditError> {
    let text = crate::mcp_keys_config::read(&home.join("config.toml"))?;
    if text.is_empty() {
        return Ok(None);
    }
    let table = text
        .parse::<toml::Table>()
        .map_err(|_| ConfigEditError::Unsupported)?;
    let list = read_rows(&table)?;
    let target = vibe_server_name(name);
    if target.is_empty() {
        return Ok(None);
    }
    let mut matching = list.into_iter().filter(|row| {
        row.get("name")
            .and_then(toml::Value::as_str)
            .is_some_and(|saved| vibe_server_name(saved) == target)
    });
    let entry = matching.next().and_then(command_entry_of);
    if matching.next().is_some() {
        return Err(ConfigEditError::Unsupported);
    }
    Ok(entry)
}

/// Whether replacing a complete Vibe config would add, change, or enable a command server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StdioChange {
    Safe,
    Unsafe,
    Uncheckable,
}

fn named_rows(doc: &toml::Table) -> Option<HashMap<&str, &toml::Table>> {
    let mut rows = HashMap::new();
    for row in read_rows(doc).ok()? {
        let name = row.get("name")?.as_str()?;
        if rows.insert(name, row).is_some() {
            return None;
        }
    }
    Some(rows)
}

/// Checks the semantic MCP rows, so changes to unrelated Vibe settings remain allowed.
pub(crate) fn stdio_change(before: &[u8], after: &[u8]) -> StdioChange {
    let parse = |bytes: &[u8]| -> Option<toml::Table> {
        std::str::from_utf8(bytes).ok()?.parse::<toml::Table>().ok()
    };
    let (Some(before), Some(after)) = (parse(before), parse(after)) else {
        return StdioChange::Uncheckable;
    };
    let (Some(old_rows), Some(new_rows)) = (named_rows(&before), named_rows(&after)) else {
        return StdioChange::Uncheckable;
    };
    for (name, new) in new_rows {
        if new.get("transport").and_then(toml::Value::as_str) != Some("stdio") {
            continue;
        }
        let Some(old) = old_rows.get(name) else {
            return StdioChange::Unsafe;
        };
        if old.get("transport").and_then(toml::Value::as_str) != Some("stdio") {
            return StdioChange::Unsafe;
        }
        let off = |row: &toml::Table| match row.get("disabled") {
            None => Some(false),
            Some(value) => value.as_bool(),
        };
        let (Some(was_off), Some(now_off)) = (off(old), off(new)) else {
            return StdioChange::Uncheckable;
        };
        if was_off && !now_off {
            return StdioChange::Unsafe;
        }
        let (mut old, mut new) = ((*old).clone(), (*new).clone());
        old.remove("disabled");
        new.remove("disabled");
        if old != new {
            return StdioChange::Unsafe;
        }
    }
    StdioChange::Safe
}

/// Sets or clears `disabled` on command servers: `decide(name, disabled_now)` returns the new state or `None` to leave
/// the entry alone. Returns the names that changed. Entries of other transports are never touched.
///
/// # Errors
/// [`ConfigEditError`].
pub(crate) fn update_disabled(
    home: &Path,
    decide: impl Fn(&str, bool) -> Option<bool>,
) -> Result<Vec<String>, ConfigEditError> {
    let changed = RefCell::new(Vec::new());
    edit_file(home, |doc: &mut DocumentMut| {
        changed.borrow_mut().clear();
        let mut list = edit_rows(doc)?;
        list.for_each(|mut row| {
            if row.get_str("transport") != Some("stdio") {
                return Ok(false);
            }
            let name = row
                .get_str("name")
                .ok_or(ConfigEditError::Unsupported)?
                .to_owned();
            let disabled = row.get_bool("disabled") == Some(true);
            match decide(&name, disabled) {
                Some(true) if !disabled => {
                    row.set("disabled", Value::from(true));
                }
                Some(false) if disabled => {
                    row.remove("disabled");
                }
                _ => return Ok(false),
            }
            changed.borrow_mut().push(name);
            Ok(true)
        })
    })?;
    Ok(changed.into_inner())
}

#[cfg(test)]
mod normalized_name_tests {
    use super::{StdioChange, command_entry_checked, stdio_change};

    #[test]
    fn aliases_match_the_saved_command_and_collisions_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let first =
            "[[mcp_servers]]\nname = 'local-stdio'\ntransport = 'stdio'\ncommand = '/bin/sh'\n";
        std::fs::write(home.path().join("config.toml"), first).unwrap();
        assert!(
            command_entry_checked(home.path(), "local-stdio-")
                .unwrap()
                .is_some()
        );
        std::fs::write(
            home.path().join("config.toml"),
            format!("{first}[[mcp_servers]]\nname = 'local-stdio-'\ntransport = 'stdio'\ncommand = '/bin/sh'\n"),
        )
        .unwrap();
        assert!(command_entry_checked(home.path(), "local-stdio-").is_err());
    }

    #[test]
    fn toggle_inspection_refuses_unclassifiable_inline_row() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "mcp_servers = [{ name = 'local', command = '/bin/echo' }]\n",
        )
        .unwrap();
        assert!(command_entry_checked(home.path(), "local").is_err());
    }

    #[test]
    fn config_proposal_refuses_an_unclassifiable_inline_row() {
        assert_eq!(
            stdio_change(
                b"",
                b"mcp_servers = [{ name = 'local', command = '/bin/echo' }]\n"
            ),
            StdioChange::Uncheckable
        );
    }
}
