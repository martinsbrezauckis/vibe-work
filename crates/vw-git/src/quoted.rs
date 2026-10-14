//! Quoting for prepared git messages. Depends on `serde_json`; called by pr, checklog and merge.
//! Repository and forge fields cannot introduce new request lines; multiline data has no closable code fence.

/// A field as a JSON string: quotes, newlines and ASCII controls stay inside the value.
pub(crate) fn field(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

/// Every line, including blank lines, is visibly quoted. A bare carriage return cannot start an unquoted line.
pub(crate) fn lines(value: &str) -> String {
    value
        .replace('\r', "\\r")
        .split('\n')
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
