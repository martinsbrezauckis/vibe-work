//! Bounded browser key chords, shared by argument validation and the page request assembler.
//! Depends on the action key vocabulary; never represents a system-wide keyboard shortcut.
use serde_json::{Value, json};

pub(crate) fn request(chord: &str) -> Option<Value> {
    if chord.len() > 80 {
        return None;
    }
    let mut parts: Vec<_> = chord.split('+').collect();
    let key = if chord == "+" { "+" } else { parts.pop()? };
    if chord == "+" {
        parts.clear();
    }
    if !(crate::act::KEYS.contains(&key)
        || (key.chars().count() == 1 && !key.chars().any(char::is_control)))
    {
        return None;
    }
    let mut flags = [false; 4];
    for part in parts {
        let index = match part {
            "Control" | "Ctrl" => 0,
            "Shift" => 1,
            "Alt" => 2,
            "Meta" => 3,
            _ => return None,
        };
        if flags[index] {
            return None;
        }
        flags[index] = true;
    }
    Some(json!({
        "key": if key == "Space" { " " } else { key },
        "modifiers": {"ctrlKey": flags[0], "shiftKey": flags[1], "altKey": flags[2], "metaKey": flags[3]}
    }))
}
