//! Bounded rendered datasets carried by browser observations and consumed by the chat-folder exporter.
//! Selectors are read-only DOM queries; rows never contain native form values or raw site storage.
use serde::{Deserialize, Serialize};

/// One bounded extraction request; table index is selected after native frame merging.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtractOptions {
    /// Table, repeated list items or links.
    pub kind: ExtractKind,
    /// Optional CSS selector for table roots or repeated items.
    pub selector: Option<String>,
    /// Optional schema. Tables rename columns; lists/links may use relative selectors.
    #[serde(default)]
    pub columns: Vec<Column>,
    /// Zero-based table index across the main document and readable child frames.
    #[serde(default)]
    pub index: usize,
    /// Override first-row headers for captured connected-browser table rows.
    pub header: Option<bool>,
}

/// Supported rendered datasets.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExtractKind {
    /// A native HTML table.
    Table,
    /// Repeated rendered elements.
    List,
    /// Visible link text and destination.
    Links,
}

/// One named output column; values are text or resolved href only.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// Spreadsheet heading.
    pub name: String,
    /// Optional relative CSS selector.
    pub selector: Option<String>,
    /// Text (default) or href.
    pub attribute: Option<String>,
}

impl ExtractOptions {
    /// Validates bounded queries and rejects unsupported extraction instead of guessing.
    /// # Errors
    /// A reason and correction for an invalid schema or selector.
    pub fn validate(&self) -> Result<(), String> {
        if self.index >= 64
            || self.columns.len() > 32
            || self
                .selector
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 2000)
        {
            return Err("Extraction exceeds its selector, table index or 32-column limit; narrow the request.".into());
        }
        for column in &self.columns {
            if column.name.is_empty()
                || column.name.len() > 200
                || column
                    .selector
                    .as_ref()
                    .is_some_and(|s| s.is_empty() || s.len() > 2000)
                || !matches!(column.attribute.as_deref(), None | Some("text" | "href"))
                || (self.kind == ExtractKind::Table
                    && (column.selector.is_some() || column.attribute.is_some()))
            {
                return Err("Invalid extraction schema; use named columns, and text or href relative selectors only for lists/links.".into());
            }
        }
        Ok(())
    }
}

/// A single page/frame's dataset; all contents remain untrusted page data.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Dataset {
    /// Column headings in row order.
    pub columns: Vec<String>,
    /// Rendered cells, at most 500 rows per page.
    pub rows: Vec<Vec<String>>,
    /// Redacted page URL.
    pub source: String,
    /// True when the scan, row count or cell bounds omitted content.
    pub truncated: bool,
}
