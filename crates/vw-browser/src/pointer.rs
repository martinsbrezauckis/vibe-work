//! Parses everyday click variants and modifiers, preserving the existing checked ref/coordinate action.
//! Called by the action parser and native point preparation; all variants share the same input guard.
use crate::Action;
use serde_json::Value;
use vw_preview::ClickOptions;

pub(crate) fn parse(args: &Value) -> Result<Action, String> {
    let (button, count) = match args["action"].as_str() {
        Some("move" | "wheel") => (0, 0),
        Some("page.double_click") => (0, 2),
        Some("page.triple_click") => (0, 3),
        Some("page.right_click") => (2, 1),
        Some("page.middle_click") => (1, 1),
        _ => (0, 1),
    };
    let wheel = if args["action"] == "wheel" {
        let delta = |key: &str| {
            args.get(key).map_or(Some(0), Value::as_i64)
            .filter(|n| (-20_000..=20_000).contains(n)).and_then(|n| i32::try_from(n).ok())
            .ok_or("Wheel deltas must be integers from -20000 to 20000. Choose bounded deltaX and deltaY.")
        };
        let deltas = (delta("deltaX")?, delta("deltaY")?);
        if deltas == (0, 0) {
            return Err(
                "Wheel needs a nonzero deltaX or deltaY. Choose the intended scroll direction."
                    .into(),
            );
        }
        Some(deltas)
    } else {
        None
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
    if count == 0 && modifiers != 0 {
        return Err(
            "Pointer motion does not hold modifier keys. Omit modifiers and try again.".into(),
        );
    }
    plain["action"] = Value::String(
        if count == 0 && args.get("x").is_none() && args.get("y").is_none() {
            "hover"
        } else {
            "click"
        }
        .into(),
    );
    if let Some(map) = plain.as_object_mut() {
        map.remove("modifiers");
        map.remove("deltaX");
        map.remove("deltaY");
    }
    Ok(Action::Pointer {
        base: Box::new(crate::act::parse_page(&plain)?),
        options: ClickOptions {
            button,
            count,
            modifiers,
            wheel,
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
    if options.wheel.is_some() {
        return "wheel";
    }
    match (options.button, options.count) {
        (_, 0) => "hover",
        (1, _) => "page.middle_click",
        (2, _) => "page.right_click",
        (_, 2) => "page.double_click",
        (_, 3) => "page.triple_click",
        _ => "click",
    }
}
