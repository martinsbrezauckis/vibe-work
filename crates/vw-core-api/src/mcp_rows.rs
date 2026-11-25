//! The two TOML representations of Vibe's `mcp_servers`: `[[mcp_servers]]` and an array of inline tables.
//! Both readers and editors validate every row before a command-server decision; an unclassifiable row refuses
//! the operation. `toml_edit` keeps untouched fields and the rest of the config as written.
//!
//! Depends on `toml`, `toml_edit` and `mcp_keys_config::ConfigEditError`. Called by command-server safety checks,
//! command pause, connector views and MCP config writers.

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use crate::mcp_keys_config::ConfigEditError;

fn validate(
    name: Option<&str>,
    transport: Option<&str>,
    command: Option<&str>,
    disabled_present: bool,
    disabled: Option<bool>,
) -> Result<(), ConfigEditError> {
    if name.is_none_or(str::is_empty) || (disabled_present && disabled.is_none()) {
        return Err(ConfigEditError::Unsupported);
    }
    match transport {
        Some("stdio") if command.is_some_and(|value| !value.trim().is_empty()) => Ok(()),
        Some("http" | "streamable-http") => Ok(()),
        _ => Err(ConfigEditError::Unsupported),
    }
}

/// Validated semantic rows, independent of which TOML array representation Vibe wrote.
pub(crate) fn read_rows(doc: &toml::Table) -> Result<Vec<&toml::Table>, ConfigEditError> {
    let Some(item) = doc.get("mcp_servers") else {
        return Ok(Vec::new());
    };
    let list = item.as_array().ok_or(ConfigEditError::Unsupported)?;
    list.iter()
        .map(|value| {
            let row = value.as_table().ok_or(ConfigEditError::Unsupported)?;
            validate(
                row.get("name").and_then(toml::Value::as_str),
                row.get("transport").and_then(toml::Value::as_str),
                row.get("command").and_then(toml::Value::as_str),
                row.contains_key("disabled"),
                row.get("disabled").and_then(toml::Value::as_bool),
            )?;
            Ok(row)
        })
        .collect()
}

/// One mutable row, retaining its original TOML representation.
pub(crate) enum RowMut<'a> {
    Table(&'a mut Table),
    Inline(&'a mut InlineTable),
}

impl RowMut<'_> {
    pub(crate) fn get_str(&self, key: &str) -> Option<&str> {
        match self {
            Self::Table(row) => row.get(key).and_then(Item::as_str),
            Self::Inline(row) => row.get(key).and_then(Value::as_str),
        }
    }

    pub(crate) fn get_bool(&self, key: &str) -> Option<bool> {
        match self {
            Self::Table(row) => row.get(key).and_then(Item::as_bool),
            Self::Inline(row) => row.get(key).and_then(Value::as_bool),
        }
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        match self {
            Self::Table(row) => row.contains_key(key),
            Self::Inline(row) => row.contains_key(key),
        }
    }

    fn validate(&self) -> Result<(), ConfigEditError> {
        validate(
            self.get_str("name"),
            self.get_str("transport"),
            self.get_str("command"),
            self.contains("disabled"),
            self.get_bool("disabled"),
        )
    }

    pub(crate) fn set(&mut self, key: &str, value: Value) {
        match self {
            Self::Table(row) => {
                row.insert(key, Item::Value(value));
            }
            Self::Inline(row) => {
                if !row.contains_key(key)
                    && let Some(last) = row.iter().last().map(|(last, _)| last.to_owned())
                    && let Some(previous) = row.get_mut(&last)
                {
                    // Vibe's inline writer leaves a space before `}`. On append, that space belongs
                    // after the new value; otherwise toml_edit puts it before the new comma.
                    previous.decor_mut().set_suffix("");
                }
                row.insert(key, value);
            }
        }
    }

    pub(crate) fn remove(&mut self, key: &str) {
        match self {
            Self::Table(row) => {
                row.remove(key);
            }
            Self::Inline(row) => {
                let last = row.iter().last().map(|(last, _)| last.to_owned());
                let suffix = row
                    .get(key)
                    .and_then(|value| value.decor().suffix())
                    .cloned();
                row.remove(key);
                if last.as_deref() == Some(key)
                    && let Some(previous) = row.iter().last().map(|(name, _)| name.to_owned())
                    && let Some(value) = row.get_mut(&previous)
                {
                    value
                        .decor_mut()
                        .set_suffix(suffix.unwrap_or_else(|| " ".into()));
                }
            }
        }
    }
}

/// Mutable list of rows in its original representation.
pub(crate) enum RowsMut<'a> {
    Tables(&'a mut ArrayOfTables),
    Inline(&'a mut Array),
}

impl RowsMut<'_> {
    /// Visits all rows; a malformed one aborts before the atomic editor writes anything.
    pub(crate) fn for_each(
        &mut self,
        mut change: impl FnMut(RowMut<'_>) -> Result<bool, ConfigEditError>,
    ) -> Result<bool, ConfigEditError> {
        let mut changed = false;
        match self {
            Self::Tables(list) => {
                for table in list.iter_mut() {
                    let row = RowMut::Table(table);
                    row.validate()?;
                    changed |= change(row)?;
                }
            }
            Self::Inline(list) => {
                for value in list.iter_mut() {
                    let table = value
                        .as_inline_table_mut()
                        .ok_or(ConfigEditError::Unsupported)?;
                    let row = RowMut::Inline(table);
                    row.validate()?;
                    changed |= change(row)?;
                }
            }
        }
        Ok(changed)
    }

    /// Checks for an occupied name, validating all existing rows first.
    pub(crate) fn has_name(&mut self, name: &str) -> Result<bool, ConfigEditError> {
        let mut found = false;
        self.for_each(|row| {
            found |= row.get_str("name") == Some(name);
            Ok(false)
        })?;
        Ok(found)
    }

    /// Adds a new entry in the form the existing list uses.
    pub(crate) fn push(&mut self, table: Table) {
        match self {
            Self::Tables(list) => list.push(table),
            Self::Inline(list) => {
                list.push(Value::InlineTable(table.into_inline_table()));
            }
        }
    }
}

/// Opens the shared editor. A missing list begins as an array of tables; an existing list keeps its form.
pub(crate) fn edit_rows(doc: &mut DocumentMut) -> Result<RowsMut<'_>, ConfigEditError> {
    if !doc.contains_key("mcp_servers") {
        doc.insert("mcp_servers", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    match doc.get_mut("mcp_servers") {
        Some(Item::ArrayOfTables(list)) => Ok(RowsMut::Tables(list)),
        Some(Item::Value(Value::Array(list))) => Ok(RowsMut::Inline(list)),
        _ => Err(ConfigEditError::Unsupported),
    }
}
