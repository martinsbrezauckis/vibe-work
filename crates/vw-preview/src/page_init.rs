//! The browser page environment: an initialization script every page of a session's browser gets at
//! document start, next to the console script. It answers dialogs raised during an agent action, and counts DOM mutations for settling (`js/page_init.js`).
//! It also draws the page's own right-click menu, in the app's menu colours for the page's light or dark (the app's
//! theme when the page shows neither).
//!
//! Depends on: `console` (the per-mount accessor), `serde_json`. Called by: `service` (mount), vw-browser (the
//! accessor name in its scripts), tests.
//! Invariants: the script installs one non-enumerable, non-configurable object under [`page_accessor`]; it never
//! sends anything anywhere (no network, no IPC). [`MENU_TOKENS`] are the values of the app's menu tokens in
//! `ui/src/styles/tokens.css` (a test keeps them equal).

/// The script source, `__VW_NAME__`, `__VW_THEME__` and `__VW_MENU__` replaced at mount.
const SOURCE: &str = include_str!("js/page_init.js");

/// The page environment's accessor name for a mount whose console accessor is `console_accessor`.
#[must_use]
pub fn page_accessor(console_accessor: &str) -> String {
    format!("{console_accessor}_page")
}

/// The app's menu tokens the page menu uses: name, dark value, light value (`ui/src/styles/tokens.css`).
pub const MENU_TOKENS: [(&str, &str, &str); 7] = [
    ("raised", "#1e1e23", "#ffffff"),
    ("active", "#2b2b32", "#e7e5dc"),
    ("text", "#f4f4f5", "#201f1c"),
    ("text-2", "#b4b4bc", "#615f57"),
    ("line", "#ffffff10", "#00000012"),
    ("line-2", "#ffffff1f", "#0000001f"),
    (
        "shadow",
        "0 18px 50px -12px #000000aa, 0 0 0 1px var(--line-2)",
        "0 18px 40px -14px #0000002e, 0 0 0 1px var(--line-2)",
    ),
];

fn menu_palette() -> String {
    let theme = |light: bool| {
        MENU_TOKENS
            .iter()
            .map(|&(name, dark, light_value)| {
                let value = if light { light_value } else { dark };
                (name.to_owned(), serde_json::Value::String(value.to_owned()))
            })
            .collect::<serde_json::Map<_, _>>()
    };
    serde_json::json!({"dark": theme(false), "light": theme(true)}).to_string()
}

/// The initialization script for the accessor `name` (the app's theme taken as dark).
#[must_use]
pub fn page_init_script(name: &str) -> String {
    page_init_script_themed(name, "dark")
}

/// The initialization script for the accessor `name`; `theme` (`light`, else dark) is the app's, which the page
/// menu uses when the page shows no light or dark of its own.
#[must_use]
pub fn page_init_script_themed(name: &str, theme: &str) -> String {
    let theme = if theme == "light" { "light" } else { "dark" };
    SOURCE
        .replace(
            "__VW_NAME__",
            &serde_json::Value::String(name.to_owned()).to_string(),
        )
        .replace("__VW_THEME__", &format!("\"{theme}\""))
        .replace("__VW_MENU__", &menu_palette())
}

#[cfg(test)]
mod tests {
    use super::MENU_TOKENS;

    /// The value of `--name` in the block of `tokens.css` that starts at `from`.
    fn token(css: &str, from: &str, name: &str) -> String {
        let block = &css[css.find(from).expect("theme block")..];
        let at = block.find(&format!("--{name}:")).expect("token");
        block[at..]
            .split_once(':')
            .and_then(|(_, rest)| rest.split_once(';'))
            .map(|(value, _)| value.trim().to_owned())
            .expect("token value")
    }

    #[test]
    fn the_page_menu_uses_the_apps_menu_tokens() {
        let css = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/styles/tokens.css"),
        )
        .expect("tokens.css");
        for (name, dark, light) in MENU_TOKENS {
            assert_eq!(token(&css, ":root", name), dark, "dark --{name}");
            assert_eq!(
                token(&css, ":root[data-theme=\"light\"]", name),
                light,
                "light --{name}"
            );
        }
    }
}
