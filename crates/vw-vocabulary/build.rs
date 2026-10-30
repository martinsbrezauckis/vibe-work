//! Validates the shared vocabulary JSON and emits static Rust lookups at build time.
//! Depends on `serde_json`; called by Cargo. The UI reads the same JSON directly.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

use serde_json::Value;

const GROUPS: [&str; 10] = [
    "accessModes",
    "thinkingLevels",
    "vaultKinds",
    "vaultPermissions",
    "browserPermissions",
    "chromeControl",
    "locks",
    "workNames",
    "builtinPlugins",
    "builtinSkills",
];

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("vocabulary entry needs a nonempty {field}")))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=vocabulary.json");
    let source = fs::read_to_string("vocabulary.json")?;
    let root: Value = serde_json::from_str(&source)?;
    let object = root
        .as_object()
        .ok_or_else(|| invalid("vocabulary root must be an object"))?;
    let mut entries = BTreeMap::new();
    for group in GROUPS {
        let terms = object
            .get(group)
            .and_then(Value::as_object)
            .ok_or_else(|| invalid(format!("vocabulary group {group} must be an object")))?;
        for (id, value) in terms {
            entries.insert(
                (group, id.as_str()),
                (
                    required_string(value, "name")?,
                    required_string(value, "description")?,
                    required_string(value, "tooltip")?,
                ),
            );
        }
    }
    let texts = object
        .get("texts")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("vocabulary texts must be an object"))?;
    required_string(&root["texts"], "agentPausedRefusal")?;
    let mut output = String::from(
        "pub(crate) fn generated_term(group: &str, id: &str) -> Option<Term> {\n    match (group, id) {\n",
    );
    for ((group, id), (name, description, tooltip)) in entries {
        writeln!(
            output,
            "        ({group:?}, {id:?}) => Some(Term {{ name: {name:?}, description: {description:?}, tooltip: {tooltip:?} }}),"
        )?;
    }
    output.push_str("        _ => None,\n    }\n}\n");
    output.push_str(
        "// One arm per vocabulary text: its length follows the vocabulary, not a design choice.\n#[allow(clippy::too_many_lines)]\npub(crate) fn generated_text(id: &str) -> Option<&'static str> {\n    match id {\n",
    );
    for (id, value) in texts {
        let text = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("vocabulary text {id} must be a nonempty string")))?;
        writeln!(output, "        {id:?} => Some({text:?}),")?;
    }
    output.push_str("        _ => None,\n    }\n}\n");
    let path = PathBuf::from(
        std::env::var_os("OUT_DIR").ok_or_else(|| invalid("Cargo must set OUT_DIR"))?,
    );
    fs::write(path.join("vocabulary.rs"), output)?;
    Ok(())
}
