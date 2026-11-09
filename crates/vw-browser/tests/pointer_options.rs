//! Click variant argument and evidence contracts. Uses the public parser; native effects are covered on the playground.
use serde_json::json;
use vw_browser::{Action, parse};

#[test]
fn variants_keep_the_observed_target_and_validate_modifiers() {
    for (name, button, count) in [
        ("page.double_click", 0, 2),
        ("page.triple_click", 0, 3),
        ("page.right_click", 2, 1),
        ("page.middle_click", 1, 1),
    ] {
        let action =
            parse(&json!({"action":name,"ref":"e12345678","modifiers":["Control","Shift"]}))
                .unwrap();
        assert_eq!(action.name(), name);
        assert_eq!(action.reference(), Some("e12345678"));
        let Action::Pointer { base, options } = action else {
            panic!("not a native pointer action")
        };
        assert!(matches!(*base, Action::Click { .. }));
        assert_eq!(
            (options.button, options.count, options.modifiers),
            (button, count, 10)
        );
        assert!(options.valid());
        if count >= 2 {
            assert_eq!(
                options
                    .events()
                    .iter()
                    .filter(|e| **e == "dblclick")
                    .count(),
                1
            );
        }
    }
    for modifiers in [
        json!(["CapsLock"]),
        json!(["Shift", "Shift"]),
        json!("Shift"),
        json!([7]),
    ] {
        assert!(parse(&json!({"action":"click","ref":"e12345678","modifiers":modifiers})).is_err());
    }
    let action = parse(&json!({"action":"click","x":24,"y":40,"modifiers":["Alt"]})).unwrap();
    assert!(
        matches!(action,Action::Pointer{base,..} if matches!(*base,Action::ClickAt{x:24,y:40,..}))
    );
}
