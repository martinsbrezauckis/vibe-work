//! A tiny declarative shape checker for the simple privileged methods: a closed list of fields,
//! each with a kind, and nothing else allowed. This is the "default deny with a tiny value-pinned
//! allow-list" style of `vw-supervisor`'s env filter, applied to JSON params.
//!
//! Depends on: canonical, error. Called by: privileged.
//! Invariant: a key that is not listed makes the whole params invalid; `null` is accepted only
//! for optional fields.

use serde_json::{Map, Value};

use crate::canonical::is_clean_text;
use crate::error::GateError;

/// The accepted kind of one field.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// Clean text (no control characters), 1..=N characters.
    Text(usize),
    /// Clean text that is also a plain name: no path separators, not `.` or `..`.
    Name(usize),
    /// A JSON boolean.
    Bool,
    /// An integer in the inclusive range.
    Int(i64, i64),
    /// One of these exact strings.
    OneOf(&'static [&'static str]),
    /// Any non-null value; the caller validates it (arrays of ops).
    Any,
}

/// One field of a shape.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Field {
    pub(crate) key: &'static str,
    pub(crate) kind: Kind,
    pub(crate) required: bool,
}

/// Required field.
pub(crate) const fn req(key: &'static str, kind: Kind) -> Field {
    Field {
        key,
        kind,
        required: true,
    }
}

/// Optional field (absent or null).
pub(crate) const fn opt(key: &'static str, kind: Kind) -> Field {
    Field {
        key,
        kind,
        required: false,
    }
}

/// Builds the standard "invalid params" error.
pub(crate) fn invalid(method: &str, detail: impl Into<String>) -> GateError {
    GateError::InvalidParams {
        method: method.to_owned(),
        detail: detail.into(),
    }
}

/// Requires `params` to be an object whose keys are all listed in `fields`, with required ones
/// present and every value of its kind. Returns the object for further rule checks.
pub(crate) fn check<'a>(
    method: &str,
    params: &'a Value,
    fields: &[Field],
) -> Result<&'a Map<String, Value>, GateError> {
    let object = params
        .as_object()
        .ok_or_else(|| invalid(method, "params must be an object"))?;
    if let Some(extra) = object
        .keys()
        .find(|k| !fields.iter().any(|f| f.key == k.as_str()))
    {
        return Err(invalid(method, format!("unknown field `{extra}`")));
    }
    for field in fields {
        match object.get(field.key) {
            None | Some(Value::Null) if !field.required => {}
            None | Some(Value::Null) => {
                return Err(invalid(method, format!("`{}` is required", field.key)));
            }
            Some(value) => {
                if !fits(field.kind, value) {
                    return Err(invalid(
                        method,
                        format!("`{}` has the wrong type or value", field.key),
                    ));
                }
            }
        }
    }
    Ok(object)
}

fn fits(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::Text(max) => value.as_str().is_some_and(|s| is_clean_text(s, max)),
        Kind::Name(max) => value.as_str().is_some_and(|s| {
            is_clean_text(s, max) && !s.contains(['/', '\\']) && s != "." && s != ".."
        }),
        Kind::Any => true,
        Kind::Bool => value.is_boolean(),
        Kind::Int(lo, hi) => value.as_i64().is_some_and(|n| (lo..=hi).contains(&n)),
        Kind::OneOf(allowed) => value.as_str().is_some_and(|s| allowed.contains(&s)),
    }
}
