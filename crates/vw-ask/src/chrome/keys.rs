//! Portable page-key allowlist shared by all browser gateways and the native input planner.
//! Only page navigation, text entry and editing chords are admitted; no browser or OS command authority.
use serde_json::Value;

/// Named page keys accepted by every browser; adapters only translate these to native key codes.
pub const NAMED: &[&str] = &[
    "Backspace",
    "Tab",
    "Enter",
    "Escape",
    "PageUp",
    "PageDown",
    "End",
    "Home",
    "ArrowLeft",
    "ArrowUp",
    "ArrowRight",
    "ArrowDown",
    "Insert",
    "Delete",
    "Space",
    "Shift",
    "Control",
    "Alt",
    "Meta",
];

const REFUSAL: &str = "This key can invoke a browser or system shortcut outside the page. Use a named browser action or ask the user to perform that shortcut; page.key accepts page keys and editing shortcuts only.";
const INVALID: &str = "The page key or modifiers are invalid. Use one printable character or a supported named key, with each Alt, Control, Meta or Shift modifier at most once.";

fn parts<'a>(key: &'a str, modifiers: &[&'a str]) -> Result<(&'a str, u8), String> {
    let mut pieces: Vec<_> = if key == "+" {
        vec![key]
    } else {
        key.split('+').collect()
    };
    let key = pieces.pop().ok_or(INVALID)?;
    let mut flags = 0;
    for modifier in pieces.into_iter().chain(modifiers.iter().copied()) {
        let bit = match modifier {
            "Control" | "Ctrl" => 1,
            "Shift" => 2,
            "Alt" => 4,
            "Meta" | "Cmd" => 8,
            _ => return Err(INVALID.into()),
        };
        if flags & bit != 0 {
            return Err(INVALID.into());
        }
        flags |= bit;
    }
    Ok((key, flags))
}

/// Validate a page key independently of browser, trust tier or saved permission.
/// # Errors
/// Returns a reason and instruction for malformed input or a chord outside page/editing scope.
pub fn validate(key: &str, modifiers: &[&str]) -> Result<(), String> {
    if key.len() > 128 || modifiers.len() > 4 {
        return Err(INVALID.into());
    }
    let (key, flags) = parts(key, modifiers)?;
    // WebDriver and AppKit interpret these private-use ranges as special keys, not printable text.
    if key
        .chars()
        .any(|c| ('\u{e000}'..='\u{e05d}').contains(&c) || ('\u{f700}'..='\u{f8ff}').contains(&c))
    {
        return Err(REFUSAL.into());
    }
    let printable = key.chars().count() == 1 && !key.chars().any(char::is_control);
    let page = NAMED.contains(&key);
    if !page && !printable {
        return Err(if key.starts_with('F') && key[1..].parse::<u8>().is_ok() {
            REFUSAL
        } else {
            INVALID
        }
        .into());
    }
    let lower = key.to_ascii_lowercase();
    let allowed = match flags {
        0 => true,
        2 => printable || page && !matches!(key, "Escape" | "Shift" | "Control" | "Alt" | "Meta"),
        1 => {
            matches!(lower.as_str(), "a" | "c" | "x" | "v" | "z" | "y")
                || matches!(
                    key,
                    "Enter" | "Home" | "End" | "Backspace" | "Delete" | "Insert"
                )
        }
        3 => {
            matches!(lower.as_str(), "v" | "z")
                || matches!(key, "Enter" | "Home" | "End" | "Backspace" | "Delete")
        }
        8 => {
            matches!(lower.as_str(), "a" | "c" | "x" | "v" | "z")
                || matches!(
                    key,
                    "ArrowLeft"
                        | "ArrowRight"
                        | "ArrowUp"
                        | "ArrowDown"
                        | "Home"
                        | "End"
                        | "Backspace"
                        | "Delete"
                )
        }
        10 => {
            matches!(lower.as_str(), "v" | "z")
                || matches!(
                    key,
                    "ArrowLeft"
                        | "ArrowRight"
                        | "ArrowUp"
                        | "ArrowDown"
                        | "Home"
                        | "End"
                        | "Backspace"
                        | "Delete"
                )
        }
        4 | 6 => matches!(key, "ArrowUp" | "ArrowDown" | "Backspace" | "Delete"),
        _ => false,
    };
    if allowed { Ok(()) } else { Err(REFUSAL.into()) }
}

/// Produce one wire spelling for combined chords, separate modifiers and accepted aliases.
/// # Errors
/// Returns the shared scope/shape refusal before any adapter sees the request.
pub fn canonical(key: &str, modifiers: &[&str]) -> Result<(String, Vec<&'static str>), String> {
    validate(key, modifiers)?;
    let (key, flags) = parts(key, modifiers)?;
    let names = [(1, "Control"), (2, "Shift"), (4, "Alt"), (8, "Meta")]
        .into_iter()
        .filter_map(|(bit, name)| (flags & bit != 0).then_some(name))
        .collect();
    Ok((key.to_owned(), names))
}

/// Normalize a validated page-key payload for all external adapters.
/// # Errors
/// Returns a reason and instruction for invalid fields or a forbidden shortcut.
pub fn normalize_args(args: &mut Value) -> Result<(), String> {
    let (key, names) = canonical(args["key"].as_str().ok_or(INVALID)?, &modifiers(args)?)?;
    let object = args.as_object_mut().ok_or(INVALID)?;
    object.insert("key".into(), Value::String(key));
    if names.is_empty() {
        object.remove("modifiers");
    } else {
        object.insert("modifiers".into(), serde_json::json!(names));
    }
    Ok(())
}

/// Validate the key/modifiers shape carried by a browser action.
/// # Errors
/// Refuses malformed fields or any non-page chord before input preparation.
pub fn validate_args(args: &Value) -> Result<(), String> {
    let key = args["key"].as_str().ok_or(INVALID)?;
    let modifiers = modifiers(args)?;
    validate(key, &modifiers)
}

fn modifiers(args: &Value) -> Result<Vec<&str>, String> {
    let Some(value) = args.get("modifiers") else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| INVALID.to_owned())?
        .iter()
        .map(|value| value.as_str().ok_or_else(|| INVALID.to_owned()))
        .collect()
}

/// Native editing chords need an actual focused editor so Undo cannot invoke a browser menu command.
/// # Errors
/// Returns the same invalid/scope refusal as validation.
pub fn requires_editor(args: &Value) -> Result<bool, String> {
    validate_args(args)?;
    let (key, flags) = parts(args["key"].as_str().ok_or(INVALID)?, &modifiers(args)?)?;
    Ok(
        (flags & !2 != 0 || flags == 2 && matches!(key, "Insert" | "Delete"))
            && !matches!(key.to_ascii_lowercase().as_str(), "a" | "c")
            && !(flags & !2 == 4 && matches!(key, "ArrowUp" | "ArrowDown")),
    )
}

/// The system clipboard operation implied by an admitted page key, if any. Adapters use this to keep
/// clipboard access on their existing protected clipboard route instead of bypassing it with native input.
/// # Errors
/// Returns the same invalid/scope refusal as key validation.
pub fn clipboard_operation(args: &Value) -> Result<Option<&'static str>, String> {
    validate_args(args)?;
    let (key, flags) = parts(args["key"].as_str().ok_or(INVALID)?, &modifiers(args)?)?;
    let lower = key.to_ascii_lowercase();
    Ok(match (flags, lower.as_str()) {
        (1 | 8, "c") | (1, "insert") => Some("copy"),
        (1 | 8, "x") | (2, "delete") => Some("cut"),
        (1 | 3 | 8 | 10, "v") | (2, "insert") => Some("paste"),
        _ => None,
    })
}
