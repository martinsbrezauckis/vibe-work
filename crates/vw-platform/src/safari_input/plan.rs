//! Portable native event plan: validates chords and effects before any macOS call. The backend only posts this plan.
use crate::key_positions::{NO_KEY_POSITION, typed_character_key, us_key as shortcut};
use serde_json::Value;
pub(super) struct Plan {
    pub modifiers: Vec<(u16, u64)>,
    pub key: Option<u16>,
    pub text: Vec<u16>,
    pub layout: bool,
    pub button: u32,
    pub clicks: i64,
    pub drag: bool,
    pub motion: bool,
    pub wheel: Option<(i32, i32)>,
}
fn modifiers(args: &Value) -> Result<Vec<(u16, u64)>, String> {
    let mut modifiers = Vec::new();
    if let Some(values) = args.get("modifiers") {
        let values = values
            .as_array()
            .filter(|v| v.len() <= 4)
            .ok_or("Native modifiers are invalid. Use Alt, Control, Meta or Shift.")?;
        for v in values {
            let m = match v.as_str() {
                Some("Shift") => (56, 1 << 17),
                Some("Control" | "Ctrl") => (59, 1 << 18),
                Some("Alt") => (58, 1 << 19),
                Some("Meta" | "Cmd") => (55, 1 << 20),
                _ => {
                    return Err(
                        "Native modifier is unsupported. Use Alt, Control, Meta or Shift.".into(),
                    );
                }
            };
            if modifiers.contains(&m) {
                return Err("A native modifier was repeated. Supply each modifier once.".into());
            }
            modifiers.push(m);
        }
    }
    Ok(modifiers)
}
pub(super) fn plan(action: &str, args: &Value) -> Result<Plan, String> {
    if action == "page.key" {
        vw_ask::chrome::keys::validate_args(args)?;
    }
    let modifiers = modifiers(args)?;
    let mut text = Vec::new();
    let mut layout = false;
    let keycode = if action == "page.key" {
        let name = args["key"]
            .as_str()
            .ok_or("A native key is missing. Supply one key and optional modifiers.")?;
        if name == "Insert" {
            // Apple keyboards have no Insert key, and WebKit reports a PC keyboard's Insert (virtual key 114) as
            // key and code "Help". A virtual key without a position keeps the text's key "Insert" and gives the
            // page code "Unidentified" instead of a wrong key position.
            text = vec![0xf727];
            Some(NO_KEY_POSITION)
        } else if let Some(code) = named(name) {
            Some(code)
        } else if name.chars().count() == 1
            && (modifiers.is_empty()
                || !name.is_ascii() && modifiers.iter().all(|(code, _)| *code == 56))
        {
            // The text decides the character; the virtual key gives the page its `code`. A letter or digit uses its
            // US key position (code "KeyQ", "Digit7", as the other browsers report it); any other character has no
            // position (code "Unidentified"), never the "a" key's.
            text = name.encode_utf16().collect();
            Some(typed_character_key(name))
        } else if name.chars().count() == 1 {
            layout = true;
            Some(shortcut(name).ok_or("The shortcut key is unsupported. Use a named key or a supported keyboard shortcut.")?)
        } else {
            return Err("Native input requires one key and separate modifiers. Supply key and modifiers fields.".into());
        }
    } else {
        None
    };
    if ![
        "page.key",
        "page.click",
        "page.commit",
        "page.double_click",
        "page.triple_click",
        "page.right_click",
        "page.middle_click",
        "page.hover",
        "page.move",
        "page.wheel",
        "page.drag",
    ]
    .contains(&action)
    {
        return Err("This native input is unsupported. Use an advertised action.".into());
    }
    let wheel = if action == "page.wheel" {
        if args.get("deltaX").is_none() && args.get("deltaY").is_none() {
            return Err("Native wheel needs a pixel delta. Supply deltaX or deltaY.".into());
        }
        let delta = |key: &str| -> Result<i32, String> {
            let value = args
                .get(key)
                .map_or(Some(0), Value::as_i64)
                .filter(|v| (-20_000..=20_000).contains(v))
                .ok_or(
                    "Native wheel delta is invalid. Supply an integer from -20000 to 20000 pixels.",
                )?;
            i32::try_from(value)
                .map_err(|_| "Native wheel delta overflowed. Use a smaller delta.".into())
        };
        // CoreGraphics positive wheel axes run opposite to DOM wheel deltaX/deltaY.
        Some((-delta("deltaX")?, -delta("deltaY")?))
    } else {
        None
    };
    Ok(Plan {
        wheel,
        modifiers,
        key: keycode,
        text,
        layout,
        button: match action {
            "page.right_click" => 1,
            "page.middle_click" => 2,
            _ => 0,
        },
        clicks: match action {
            "page.double_click" => 2,
            "page.triple_click" => 3,
            _ => 1,
        },
        drag: action == "page.drag",
        motion: matches!(action, "page.hover" | "page.move"),
    })
}
fn named(name: &str) -> Option<u16> {
    Some(match name {
        "Enter" | "Return" => 36,
        "Tab" => 48,
        "Shift" => 56,
        "Control" => 59,
        "Alt" => 58,
        "Meta" => 55,
        "Space" | " " => 49,
        "Backspace" => 51,
        "Escape" | "Esc" => 53,
        "Home" => 115,
        "PageUp" => 116,
        "Delete" => 117,
        "End" => 119,
        "PageDown" => 121,
        "ArrowLeft" => 123,
        "ArrowRight" => 124,
        "ArrowDown" => 125,
        "ArrowUp" => 126,
        "F1" => 122,
        "F2" => 120,
        "F3" => 99,
        "F4" => 118,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F8" => 100,
        "F9" => 101,
        "F10" => 109,
        "F11" => 103,
        "F12" => 111,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    // PREDICT (before the fix): both tests below fail. Insert was posted as virtual key 0 (kVK_ANSI_A) with the
    // Insert function character, so WebKit reported key "Insert" but code "KeyA" (and Control+Insert looked like
    // Control+KeyA to a page that reads `code`); every single typed character was virtual key 0 too, so typing "q"
    // reached the page as code "KeyA". WebKit takes `code` from the virtual key and `key` from the event's text,
    // except for virtual key 114 (a PC keyboard's Insert), which it reports as key and code "Help" (Safari, macOS 27).
    #[test]
    fn insert_keeps_its_key_without_a_wrong_position_and_never_the_help_key() {
        for modifiers in [json!([]), json!(["Shift"]), json!(["Control"])] {
            let p = plan("page.key", &json!({"key":"Insert","modifiers":modifiers})).unwrap();
            assert_eq!(p.key, Some(NO_KEY_POSITION), "code \"Unidentified\"");
            assert_ne!(
                p.key,
                Some(114),
                "WebKit reports virtual key 114 as key and code \"Help\""
            );
            assert_ne!(
                p.key,
                Some(0),
                "virtual key 0 is the \"a\" key (code \"KeyA\")"
            );
            assert_eq!(p.text, vec![0xf727], "AppKit Insert function character");
            assert!(!p.layout);
            assert_eq!(p.modifiers.len(), modifiers.as_array().unwrap().len());
        }
    }
    #[test]
    fn typed_characters_use_their_own_key_position_and_keep_their_text() {
        for (key, code) in [
            ("q", 12),
            ("Q", 12),
            ("a", 0),
            ("z", 6),
            ("7", 26),
            ("0", 29),
        ] {
            let p = plan("page.key", &json!({ "key": key })).unwrap();
            assert_eq!(p.key, Some(code), "{key}");
            assert_eq!(p.text, key.encode_utf16().collect::<Vec<_>>(), "{key}");
            assert!(!p.layout, "{key}: plain text never needs the US layout");
        }
        // No letter or digit position: the text alone carries them (code "Unidentified", never "KeyA").
        for key in [",", "/", "!", "Ā", "🙂", "é"] {
            let p = plan("page.key", &json!({ "key": key })).unwrap();
            assert_eq!(p.key, Some(NO_KEY_POSITION), "{key}");
            assert_eq!(p.text, key.encode_utf16().collect::<Vec<_>>(), "{key}");
        }
        let p = plan("page.key", &json!({"key":"Ā","modifiers":["Shift"]})).unwrap();
        assert_eq!(p.key, Some(NO_KEY_POSITION));
    }
    #[test]
    fn chords_unicode_and_click_counts_have_exact_plans() {
        let p = plan("page.key", &json!({"key":"z","modifiers":["Meta","Shift"]})).unwrap();
        assert_eq!(p.key, Some(6));
        assert_eq!(p.modifiers, vec![(55, 1 << 20), (56, 1 << 17)]);
        assert!(p.layout);
        let p = plan("page.key", &json!({"key":"🙂"})).unwrap();
        assert_eq!(p.text, "🙂".encode_utf16().collect::<Vec<_>>());
        assert!(!p.layout);
        assert_eq!(plan("page.triple_click", &json!({})).unwrap().clicks, 3);
        assert_eq!(plan("page.middle_click", &json!({})).unwrap().button, 2);
        assert!(plan("page.hover", &json!({})).unwrap().motion);
        assert!(plan("page.drag", &json!({})).unwrap().drag);
    }
    #[test]
    fn shifted_characters_admitted_by_shared_policy_have_native_plans() {
        for key in [
            "!", "@", "#", "$", "%", "^", "&", "*", "(", ")", "_", "+", "{", "}", "|", ":", "\"",
            "<", ">", "?", "~", "Ā", "🙂",
        ] {
            vw_ask::chrome::keys::validate(key, &["Shift"]).unwrap();
            let p = plan("page.key", &json!({"key":key,"modifiers":["Shift"]})).unwrap();
            assert_eq!(p.modifiers, vec![(56, 1 << 17)]);
            assert!(p.key.is_some());
            if key.is_ascii() {
                assert!(p.layout);
                assert_eq!(p.text, Vec::<u16>::new());
            } else {
                assert!(!p.layout);
                assert_eq!(p.text, key.encode_utf16().collect::<Vec<_>>());
            }
            if key == "!" {
                assert_eq!(p.key, Some(18));
            }
        }
    }
    #[test]
    fn wheel_requires_bounded_pixel_deltas_and_preserves_axis_signs() {
        assert_eq!(
            plan("page.wheel", &json!({"deltaX":50,"deltaY":-120}))
                .unwrap()
                .wheel,
            Some((-50, 120))
        );
        for args in [json!({}), json!({"deltaY":20_001}), json!({"deltaX":1.5})] {
            assert!(plan("page.wheel", &args).is_err());
        }
    }
    #[test]
    fn page_keys_are_portable_and_native_plans_refuse_browser_shortcuts() {
        for key in [
            "Shift",
            "Control",
            "Alt",
            "Meta",
            "Insert",
            "Enter",
            "Tab",
            "Home",
            "ArrowLeft",
        ] {
            assert!(plan("page.key", &json!({"key":key})).unwrap().key.is_some());
        }
        for key in ["q", "w", "t", "n", "Tab", "Space", "h", "m"] {
            let reason = plan("page.key", &json!({"key":key,"modifiers":["Meta"]}))
                .err()
                .unwrap();
            assert!(reason.contains("browser or system"), "{reason}");
        }
        assert!(
            plan(
                "page.key",
                &json!({"key":"q","modifiers":["Control","Meta"]})
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_chords_and_unsupported_input_refuse_before_effects() {
        for args in [
            json!({"key":"Meta+A"}),
            json!({"key":"a","modifiers":["Meta","Meta"]}),
            json!({"key":"a","modifiers":["Unknown"]}),
            json!({"key":""}),
        ] {
            assert!(plan("page.key", &args).is_err());
        }
        assert!(plan("shell.run", &json!({})).is_err());
    }
}
