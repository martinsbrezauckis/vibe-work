//! JSON Patch (RFC 6902) with Vibe's extra `append` operation, applied in place and atomically.
//!
//! Responsibility: turn a list of wire patch operations (`{"op","path","value"}`) into mutations of
//! a `serde_json::Value`. Supported ops: `add`, `replace`, `remove`, `test` (RFC 6902) and `append`
//! (Vibe extension: string suffix, `vibe/app_server/_patch.py` `_standard_operation`). `move` and
//! `copy` are not on the Vibe wire (`JsonPatchOperation.op` is a closed 5-value literal) and are
//! rejected as unknown ops.
//! Depends on: `serde_json` only. Called by: `reducer` (session and entry patches).
//! Invariant: [`apply_patch`] is atomic. If any operation fails, the document is left exactly as it
//! was before the call (an undo log reverts the operations already applied), so a bad server patch
//! can never leave a half-patched entry in the state of record.

use serde_json::Value;

/// Why a patch could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    /// The operation object lacks `op` or `path`, or they are not strings.
    #[error("malformed patch operation #{index}: {reason}")]
    Malformed {
        /// Position of the operation in the patch.
        index: usize,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// `op` is not one of add, replace, remove, test, append.
    #[error("unknown patch op {op:?} (operation #{index})")]
    UnknownOp {
        /// Position of the operation in the patch.
        index: usize,
        /// The op name as received.
        op: String,
    },
    /// The path (or its parent) does not exist in the document, or an array index is invalid.
    #[error("patch path {path:?} cannot be resolved (operation #{index})")]
    PathNotFound {
        /// Position of the operation in the patch.
        index: usize,
        /// The JSON Pointer that failed.
        path: String,
    },
    /// `test` compared the target with `value` and they differ.
    #[error("test failed at {path:?} (operation #{index})")]
    TestFailed {
        /// Position of the operation in the patch.
        index: usize,
        /// The JSON Pointer that was tested.
        path: String,
    },
    /// `append` needs a string target and a string value.
    #[error("append at {path:?} needs a string target and a string value (operation #{index})")]
    AppendType {
        /// Position of the operation in the patch.
        index: usize,
        /// The JSON Pointer that was targeted.
        path: String,
    },
}

/// One reversible step, recorded so a failing patch can be rolled back.
enum Undo {
    /// Put `old` back at `path` (replace) .
    Replace { path: String, old: Value },
    /// Remove the key / element that an `add` created.
    RemoveAdded { path: String },
    /// Restore an object key or array element that an `add` overwrote or a `remove` deleted.
    Reinsert { path: String, old: Value },
    /// Truncate a string back to `len` bytes (undo of `append`).
    Truncate { path: String, len: usize },
}

/// Applies `ops` to `doc` in place. On error `doc` is unchanged.
///
/// # Errors
/// Returns the first failing operation as a [`PatchError`]; earlier operations are rolled back.
pub fn apply_patch(doc: &mut Value, ops: &[Value]) -> Result<(), PatchError> {
    let mut undo: Vec<Undo> = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
        if let Err(err) = apply_one(doc, index, op, &mut undo) {
            rollback(doc, undo);
            return Err(err);
        }
    }
    Ok(())
}

fn rollback(doc: &mut Value, undo: Vec<Undo>) {
    for step in undo.into_iter().rev() {
        // Every undo path was valid right after the op it reverses, and later ops are already
        // reverted, so these lookups cannot miss; `let _` keeps rollback infallible by design.
        match step {
            Undo::Replace { path, old } => {
                let _ = put(doc, &path, old, true);
            }
            Undo::RemoveAdded { path } => {
                let _ = take(doc, &path);
            }
            Undo::Reinsert { path, old } => {
                let _ = put(doc, &path, old, false);
            }
            Undo::Truncate { path, len } => {
                if let Some(Value::String(s)) = doc.pointer_mut(&path) {
                    s.truncate(len);
                }
            }
        }
    }
}

fn apply_one(
    doc: &mut Value,
    index: usize,
    op: &Value,
    undo: &mut Vec<Undo>,
) -> Result<(), PatchError> {
    let kind = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or(PatchError::Malformed {
            index,
            reason: "missing string `op`",
        })?;
    let path = op
        .get("path")
        .and_then(Value::as_str)
        .ok_or(PatchError::Malformed {
            index,
            reason: "missing string `path`",
        })?;
    let value = op.get("value").cloned().unwrap_or(Value::Null);
    let not_found = || PatchError::PathNotFound {
        index,
        path: path.to_owned(),
    };
    if !path.is_empty() && !path.starts_with('/') {
        return Err(not_found());
    }
    match kind {
        "add" => op_add(doc, path, value, undo).ok_or_else(not_found),
        "replace" => op_replace(doc, path, value, undo).ok_or_else(not_found),
        "remove" => op_remove(doc, path, undo).ok_or_else(not_found),
        "append" => op_append(doc, index, path, &value, undo),
        "test" => {
            let target = doc.pointer(path).ok_or_else(not_found)?;
            if *target == value {
                Ok(())
            } else {
                Err(PatchError::TestFailed {
                    index,
                    path: path.to_owned(),
                })
            }
        }
        other => Err(PatchError::UnknownOp {
            index,
            op: other.to_owned(),
        }),
    }
}

fn op_add(doc: &mut Value, path: &str, value: Value, undo: &mut Vec<Undo>) -> Option<()> {
    if path.is_empty() {
        let old = std::mem::replace(doc, value);
        undo.push(Undo::Replace {
            path: String::new(),
            old,
        });
        return Some(());
    }
    undo.push(match put_returning_old(doc, path, value)? {
        Put::Overwrote(old) => Undo::Reinsert {
            path: path.to_owned(),
            old,
        },
        Put::Created => Undo::RemoveAdded {
            path: normalized_added_path(doc, path),
        },
    });
    Some(())
}

fn op_replace(doc: &mut Value, path: &str, value: Value, undo: &mut Vec<Undo>) -> Option<()> {
    let slot = if path.is_empty() {
        &mut *doc
    } else {
        doc.pointer_mut(path)?
    };
    let old = std::mem::replace(slot, value);
    undo.push(Undo::Replace {
        path: path.to_owned(),
        old,
    });
    Some(())
}

fn op_remove(doc: &mut Value, path: &str, undo: &mut Vec<Undo>) -> Option<()> {
    let old = take(doc, path)?;
    undo.push(Undo::Reinsert {
        path: path.to_owned(),
        old,
    });
    Some(())
}

fn op_append(
    doc: &mut Value,
    index: usize,
    path: &str,
    value: &Value,
    undo: &mut Vec<Undo>,
) -> Result<(), PatchError> {
    let target = doc
        .pointer_mut(path)
        .ok_or_else(|| PatchError::PathNotFound {
            index,
            path: path.to_owned(),
        })?;
    match (target, value) {
        (Value::String(s), Value::String(suffix)) => {
            undo.push(Undo::Truncate {
                path: path.to_owned(),
                len: s.len(),
            });
            s.push_str(suffix);
            Ok(())
        }
        _ => Err(PatchError::AppendType {
            index,
            path: path.to_owned(),
        }),
    }
}

/// What `add` did to its target slot.
enum Put {
    Created,
    Overwrote(Value),
}

/// Splits a non-root pointer into (parent pointer, unescaped final token).
fn split_pointer(path: &str) -> Option<(&str, String)> {
    let cut = path.rfind('/')?;
    let token = path[cut + 1..].replace("~1", "/").replace("~0", "~");
    Some((&path[..cut], token))
}

/// Parses an array index token. Rejects signs, leading zeros and anything non-decimal (RFC 6901).
fn parse_index(token: &str) -> Option<usize> {
    if token.is_empty() || (token.len() > 1 && token.starts_with('0')) {
        return None;
    }
    if !token.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// `add`: inserts `value` at `path`. `None` when the path cannot resolve.
fn put_returning_old(doc: &mut Value, path: &str, value: Value) -> Option<Put> {
    let (parent_path, token) = split_pointer(path)?;
    match doc.pointer_mut(parent_path)? {
        Value::Object(map) => Some(
            map.insert(token, value)
                .map_or(Put::Created, Put::Overwrote),
        ),
        Value::Array(items) => {
            let at = if token == "-" {
                items.len()
            } else {
                parse_index(&token)?
            };
            if at > items.len() {
                return None;
            }
            items.insert(at, value);
            Some(Put::Created)
        }
        _ => None,
    }
}

/// Path that `RemoveAdded` must use: a `-` array token resolves to the concrete last index.
fn normalized_added_path(doc: &Value, path: &str) -> String {
    match split_pointer(path) {
        Some((parent, token)) if token == "-" => {
            let len = doc
                .pointer(parent)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("{parent}/{}", len.saturating_sub(1))
        }
        _ => path.to_owned(),
    }
}

/// Removes and returns the value at a non-root pointer.
fn take(doc: &mut Value, path: &str) -> Option<Value> {
    let (parent_path, token) = split_pointer(path)?;
    match doc.pointer_mut(parent_path)? {
        Value::Object(map) => map.remove(&token),
        Value::Array(items) => {
            let at = parse_index(&token)?;
            (at < items.len()).then(|| items.remove(at))
        }
        _ => None,
    }
}

/// Undo helper: writes `value` at `path`; `overwrite` selects replace-in-place over insert.
fn put(doc: &mut Value, path: &str, value: Value, overwrite: bool) -> Option<()> {
    if path.is_empty() {
        *doc = value;
        return Some(());
    }
    if overwrite {
        *doc.pointer_mut(path)? = value;
        return Some(());
    }
    put_returning_old(doc, path, value).map(|_| ())
}
