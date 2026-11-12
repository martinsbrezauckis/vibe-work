//! The browser page environment: an initialization script every page of a session's browser gets at
//! document start, next to the console script. Native dialogs belong to the engine; this script counts DOM mutations for settling (`js/page_init.js`).
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
const SHARE: &str = include_str!("js/page_share.js");
const SELECTION: &str = include_str!("js/page_selection.js");
const WIDGET_FORMATS: &str = include_str!("js/widget_formats.js");

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
    let environment = SOURCE
        .replace("__VW_SELECTION__", SELECTION)
        .replace(
            "const RESTORE_SELECTION = false;",
            if cfg!(target_os = "macos") {
                "const RESTORE_SELECTION = true;"
            } else {
                "const RESTORE_SELECTION = false;"
            },
        )
        .replace("__VW_SHARE__", SHARE)
        .replace(
            "__VW_NAME__",
            &serde_json::Value::String(name.to_owned()).to_string(),
        )
        .replace("__VW_THEME__", &format!("\"{theme}\""))
        .replace("__VW_MENU__", &menu_palette());
    format!("{environment}\n{WIDGET_FORMATS}")
}
