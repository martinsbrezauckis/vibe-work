//! Portable native event plan: validates chords and effects before any macOS call. The backend only posts this plan.
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
            // macOS has no Insert virtual key: 114 is Help. Use its distinct AppKit function character.
            text = vec![0xf727];
            Some(0)
        } else if let Some(code) = named(name) {
            Some(code)
        } else if name.chars().count() == 1
            && (modifiers.is_empty()
                || !name.is_ascii() && modifiers.iter().all(|(code, _)| *code == 56))
        {
            text = name.encode_utf16().collect();
            Some(0)
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
fn shortcut(name: &str) -> Option<u16> {
    Some(match name.to_ascii_lowercase().as_str() {
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" | "!" => 18,
        "2" | "@" => 19,
        "3" | "#" => 20,
        "4" | "$" => 21,
        "6" | "^" => 22,
        "5" | "%" => 23,
        "=" | "+" => 24,
        "9" | "(" => 25,
        "7" | "&" => 26,
        "-" | "_" => 27,
        "8" | "*" => 28,
        "0" | ")" => 29,
        "]" | "}" => 30,
        "o" => 31,
        "u" => 32,
        "[" | "{" => 33,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "'" | "\"" => 39,
        "k" => 40,
        ";" | ":" => 41,
        "\\" | "|" => 42,
        "," | "<" => 43,
        "/" | "?" => 44,
        "n" => 45,
        "m" => 46,
        "." | ">" => 47,
        "`" | "~" => 50,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn insert_never_posts_the_native_help_key() {
        for modifiers in [json!([]), json!(["Shift"]), json!(["Control"])] {
            let p = plan("page.key", &json!({"key":"Insert","modifiers":modifiers})).unwrap();
            assert_ne!(p.key, Some(114), "macOS code 114 is Help, not Insert");
            assert_eq!(p.text, vec![0xf727], "AppKit Insert function character");
            assert!(!p.layout);
            assert_eq!(p.modifiers.len(), modifiers.as_array().unwrap().len());
        }
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
