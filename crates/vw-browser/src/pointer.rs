//! Parses everyday click variants and modifiers, preserving the existing checked ref/coordinate action.
//! Called by the action parser and native point preparation; all variants share the same input guard.
use crate::Action;
use serde_json::Value;
use vw_preview::ClickOptions;

pub(crate) fn parse(args: &Value) -> Result<Action, String> {
    let (button, count) = match args["action"].as_str() {
        Some("page.double_click") => (0, 2),
        Some("page.triple_click") => (0, 3),
        Some("page.right_click") => (2, 1),
        Some("page.middle_click") => (1, 1),
        _ => (0, 1),
    };
    let mut modifiers = 0;
    if let Some(values) = args.get("modifiers") {
        for value in values
            .as_array()
            .filter(|a| a.len() <= 4)
            .ok_or("modifiers must be an array of Alt, Control, Meta or Shift.")?
        {
            let bit = match value.as_str() {
                Some("Alt") => 1,
                Some("Control") => 2,
                Some("Meta") => 4,
                Some("Shift") => 8,
                _ => return Err("Unknown click modifier; use Alt, Control, Meta or Shift.".into()),
            };
            if modifiers & bit != 0 {
                return Err("Click modifiers must not repeat.".into());
            }
            modifiers |= bit;
        }
    }
    let mut plain = args.clone();
    plain["action"] = Value::String("click".into());
    if let Some(map) = plain.as_object_mut() {
        map.remove("modifiers");
    }
    Ok(Action::Pointer {
        base: Box::new(crate::act::parse_page(&plain)?),
        options: ClickOptions {
            button,
            count,
            modifiers,
        },
    })
}

impl Action {
    pub(crate) fn pointer_base(&self) -> &Self {
        if let Self::Pointer { base, .. } = self {
            base
        } else {
            self
        }
    }
    pub(crate) fn click_options(&self) -> ClickOptions {
        if let Self::Pointer { options, .. } = self {
            *options
        } else if matches!(self, Self::Hover { .. }) {
            ClickOptions::HOVER
        } else {
            ClickOptions::default()
        }
    }
}

pub(crate) fn name(options: ClickOptions) -> &'static str {
    match (options.button, options.count) {
        (1, _) => "page.middle_click",
        (2, _) => "page.right_click",
        (_, 2) => "page.double_click",
        (_, 3) => "page.triple_click",
        _ => "click",
    }
}
