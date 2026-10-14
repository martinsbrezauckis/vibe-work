//! The design capture's judgement: from what the page script measured on the live page (`js/design.js`:
//! colours weighted by text length, area and border length; type samples; spacing, radius, shadow and motion values;
//! text/backdrop pairs; component samples; root custom properties) to named tokens: colour roles, a type scale, a
//! spacing scale with its base unit, radii, elevation, component tokens and contrast warnings. `design_out` writes
//! them as `DESIGN.md` (Google's DESIGN.md format, alpha) and `tokens.json` (W3C Design Tokens 2025.10).
//!
//! Role heuristics adapted from ShellX Canvas (semantic role score: accent /
//! brand / primary / cta words, background, ink/text/fg, muted, border, danger; type role: display / heading /
//! subheading / body / caption), re-weighted by on-screen use instead of rule counts.
//!
//! Depends on: serde, `colour`, `guard`. Called by: `capture`, tests (through [`derive`]).
//! Invariants:
//! - Every page-made string that reaches a token (font family, CSS variable name, element path) passes [`safe_name`]:
//!   a strict character set, a length cap, and the prompt guard (text that instructs is withheld).
//! - Colour values are always `#rrggbb[aa]` produced by [`Rgba`]; nothing the page wrote is copied as a colour.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::colour::{Rgba, contrast};
use crate::guard;

/// One weighted value the page script counted.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Row {
    /// The value (a colour, a length, a shadow).
    pub value: String,
    /// Use weight (characters, area or border length).
    pub weight: f64,
    /// How many elements.
    pub count: u64,
    /// Where it was seen (tag names, property names).
    pub contexts: Vec<String>,
}

/// A root CSS custom property.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Var {
    /// `--name`.
    pub name: String,
    /// The value the root resolves.
    pub value: String,
    /// The value as a colour, when it is one.
    pub colour: Option<String>,
}

/// One type sample (family, size, weight, line height, letter spacing, case) with its use.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FontRow {
    /// The family stack.
    pub family: String,
    /// `16px`.
    pub size: String,
    /// `400`.
    pub weight: String,
    /// `24px` or `normal`.
    pub line_height: String,
    /// `0` or a length.
    pub letter_spacing: String,
    /// `none`, `uppercase`, ...
    pub transform: String,
    /// Characters set in it.
    pub chars: f64,
    /// Tags it was seen on.
    pub tags: Vec<String>,
}

/// A text colour on its backdrop.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pair {
    /// Text colour.
    pub fg: String,
    /// Backdrop colour.
    pub bg: String,
    /// Large text (24px, or 18.66px bold).
    pub large: bool,
    /// Characters.
    pub weight: f64,
    /// Where (the last three DOM path segments).
    #[serde(rename = "where")]
    pub place: String,
}

/// A component sample (the most common styles of buttons, inputs, cards, links).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Component {
    /// Background.
    pub background_color: Option<String>,
    /// Text colour.
    pub text_color: Option<String>,
    /// Corner radius.
    pub rounded: String,
    /// `12px 16px`.
    pub padding: String,
    /// Height.
    pub height: String,
    /// Font size.
    pub font_size: String,
    /// Font weight.
    pub font_weight: String,
    /// Shadow.
    pub shadow: Option<String>,
    /// How many.
    pub count: u64,
}

/// Page facts of the capture.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageFacts {
    /// The body's (else the root's) background.
    pub background: Option<String>,
    /// `color-scheme`.
    pub color_scheme: String,
    /// The engine prefers dark.
    pub prefers_dark: bool,
    /// Elements measured.
    pub elements_seen: u64,
    /// The walk was cut.
    pub truncated: bool,
    /// Stylesheets read.
    pub sheets_read: u64,
    /// Stylesheets the page did not let us read (other origins).
    pub sheets_blocked: u64,
}

/// What `js/design.js` answers.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    /// The page's address.
    pub url: String,
    /// The page's title.
    pub title: String,
    /// Page facts.
    pub page: PageFacts,
    /// Root custom properties.
    pub vars: Vec<Var>,
    /// Text colours.
    pub text: Vec<Row>,
    /// Link colours.
    pub links: Vec<Row>,
    /// Backgrounds.
    pub backgrounds: Vec<Row>,
    /// Borders.
    pub borders: Vec<Row>,
    /// Type samples.
    pub fonts: Vec<FontRow>,
    /// Spacing values.
    pub spacing: Vec<Row>,
    /// Radii.
    pub radii: Vec<Row>,
    /// Shadows.
    pub shadows: Vec<Row>,
    /// Transitions.
    pub motion: Vec<Row>,
    /// Text/backdrop pairs.
    pub contrast: Vec<Pair>,
    /// Component samples by kind (`button`, `input`, `card`, `link`).
    pub components: BTreeMap<String, Vec<Component>>,
}

/// A named colour with why it got its role.
#[derive(Clone, Debug, PartialEq)]
pub struct ColourToken {
    /// Token name (`primary`, `neutral`, `on-surface`, ...).
    pub name: String,
    /// The colour.
    pub value: Rgba,
    /// Why (Vibe Work's words).
    pub why: String,
}

/// A typography level.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeToken {
    /// `headline-lg`, `body-md`, ...
    pub name: String,
    /// The first family of the stack (safe name).
    pub family: String,
    /// The whole stack (safe names).
    pub stack: Vec<String>,
    /// Pixels.
    pub size: f64,
    /// 100..=900.
    pub weight: u32,
    /// A multiple of the size, when known.
    pub line_height: Option<f64>,
    /// Pixels, when not 0.
    pub letter_spacing: Option<f64>,
    /// Uppercase labels.
    pub uppercase: bool,
}

/// A contrast warning.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// Text colour.
    pub fg: Rgba,
    /// Backdrop.
    pub bg: Rgba,
    /// The ratio.
    pub ratio: f64,
    /// `fail` (< 3:1) or `warning`.
    pub level: &'static str,
    /// Large text.
    pub large: bool,
    /// Where (a safe element path).
    pub place: String,
    /// Characters affected.
    pub weight: f64,
}

/// The design system as derived.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    /// Colour roles, in the DESIGN.md order.
    pub colours: Vec<ColourToken>,
    /// The palette by use: colour and its shares (0..=1) of the text (by characters), the painted area and the
    /// drawn border length; most used first.
    pub palette: Vec<(Rgba, [f64; 3])>,
    /// Colour custom properties: safe name, colour.
    pub colour_vars: Vec<(String, Rgba)>,
    /// Type levels.
    pub typography: Vec<TypeToken>,
    /// Spacing scale (name, px), smallest first.
    pub spacing: Vec<(String, f64)>,
    /// The base unit in px (8, 4 or the most common small value).
    pub base: f64,
    /// Radii (name, CSS value).
    pub rounded: Vec<(String, String)>,
    /// Shadows (computed CSS), most used first.
    pub shadows: Vec<String>,
    /// Transitions (`0.2s ease`), most used first.
    pub motion: Vec<String>,
    /// Component tokens: name, (property, value) in order.
    pub components: Vec<(String, Vec<(String, String)>)>,
    /// Contrast warnings, most text first.
    pub warnings: Vec<Warning>,
    /// `light` or `dark`.
    pub theme: &'static str,
    /// The page has no chromatic colour (black, white and greys only).
    pub monochrome: bool,
}

/// A page-made name made safe for a token file: letters, digits, space, `-`, `_`, `.`, `#`, `>` and `:` only, at
/// most `max` characters, `None` when nothing is left or the prompt guard reads it as an instruction.
#[must_use]
pub fn safe_name(text: &str, max: usize) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.#>:".contains(*c))
        .take(max)
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() || guard::instructs(&cleaned) {
        None
    } else {
        Some(cleaned)
    }
}

/// `16px` / `1.5rem` (root 16px) as pixels.
#[must_use]
pub fn px(text: &str) -> Option<f64> {
    let t = text.trim();
    let (number, factor) = if let Some(n) = t.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix("rem") {
        (n, 16.0)
    } else {
        (t, 1.0)
    };
    number
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v * factor)
}

fn colour(text: &str) -> Option<Rgba> {
    Rgba::parse(text)
}

/// Derives the tokens from the page's measurements.
#[must_use]
pub fn derive(stats: &Stats) -> Tokens {
    let mut tokens = Tokens {
        theme: "light",
        ..Tokens::default()
    };
    super::design_roles::colours(stats, &mut tokens);
    super::design_roles::typography(stats, &mut tokens);
    scales(stats, &mut tokens);
    super::design_roles::components(stats, &mut tokens);
    tokens.warnings = warnings(stats);
    tokens
}

/// Spacing (base unit + scale), radii, shadows, motion.
fn scales(stats: &Stats, tokens: &mut Tokens) {
    let mut values: Vec<(f64, f64)> = stats
        .spacing
        .iter()
        .filter_map(|r| px(&r.value).map(|v| (v, r.weight.max(1.0))))
        .filter(|(v, _)| *v >= 1.0 && *v <= 256.0)
        .collect();
    let total: f64 = values.iter().map(|(_, w)| w).sum();
    let share = |unit: f64| -> f64 {
        if total <= 0.0 {
            return 0.0;
        }
        values
            .iter()
            .filter(|(v, _)| (v / unit - (v / unit).round()).abs() < 0.01)
            .map(|(_, w)| w)
            .sum::<f64>()
            / total
    };
    tokens.base = if total <= 0.0 || share(8.0) >= 0.6 {
        8.0
    } else if share(4.0) >= 0.6 {
        4.0
    } else {
        values
            .iter()
            .filter(|(v, _)| *v <= 16.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(4.0, |(v, _)| *v)
    };
    let unit_ruled = total > 0.0 && (share(8.0) >= 0.6 || share(4.0) >= 0.6);
    values.sort_by(|a, b| b.1.total_cmp(&a.1));
    // On a page with a clear unit the scale keeps its multiples of half the unit (one-off values such as a browser
    // default button's 1px padding are noise for a token scale); otherwise every common value counts.
    let half = tokens.base / 2.0;
    values.retain(|(v, _)| {
        *v >= 2.0 && (!unit_ruled || (v / half - (v / half).round()).abs() < 0.01)
    });
    let mut scale: Vec<f64> = Vec::new();
    for (v, _) in &values {
        let v = (v * 10.0).round() / 10.0;
        if v <= 160.0 && !scale.iter().any(|s| (s - v).abs() < 0.5) {
            scale.push(v);
        }
        if scale.len() >= 6 {
            break;
        }
    }
    scale.sort_by(f64::total_cmp);
    let names: &[&str] = match scale.len() {
        1 => &["md"],
        2 => &["sm", "md"],
        3 => &["sm", "md", "lg"],
        4 => &["xs", "sm", "md", "lg"],
        5 => &["xs", "sm", "md", "lg", "xl"],
        _ => &["xs", "sm", "md", "lg", "xl", "2xl"],
    };
    tokens.spacing = scale
        .iter()
        .zip(names)
        .map(|(v, n)| ((*n).to_owned(), *v))
        .collect();

    let mut radii: Vec<(f64, String, f64)> = Vec::new();
    let mut full = false;
    for r in &stats.radii {
        if r.value.ends_with('%') || px(&r.value).is_some_and(|v| v >= 999.0) {
            full = true;
        } else if let Some(v) = px(&r.value).filter(|v| *v > 0.0) {
            radii.push((v, format!("{}px", trim_number(v)), r.weight));
        }
    }
    radii.sort_by(|a, b| b.2.total_cmp(&a.2));
    radii.truncate(3);
    radii.sort_by(|a, b| a.0.total_cmp(&b.0));
    let names: &[&str] = match radii.len() {
        1 => &["md"],
        2 => &["sm", "md"],
        _ => &["sm", "md", "lg"],
    };
    tokens.rounded = radii
        .into_iter()
        .zip(names)
        .map(|((_, value, _), name)| ((*name).to_owned(), value))
        .collect();
    if full {
        tokens
            .rounded
            .push(("full".to_owned(), "9999px".to_owned()));
    }
    tokens.shadows = stats
        .shadows
        .iter()
        .filter_map(|r| safe_css_value(&r.value))
        .take(3)
        .collect();
    tokens.motion = stats
        .motion
        .iter()
        .filter_map(|r| safe_css_value(&r.value).filter(|v| v.len() <= 80 && !guard::instructs(v)))
        .take(3)
        .collect();
}

/// A computed shadow or transition kept only with CSS value characters (including function punctuation).
fn safe_css_value(text: &str) -> Option<String> {
    let ok = text.len() <= 200
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " .,()-#%".contains(c));
    ok.then(|| text.to_owned())
}

/// Numbers without a trailing `.0`.
#[must_use]
pub fn trim_number(v: f64) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}", rounded.round())
    } else {
        format!("{rounded}")
    }
}

/// Text below 4.5:1 (3:1 for large text), most text first, at most 8.
fn warnings(stats: &Stats) -> Vec<Warning> {
    let mut out: Vec<Warning> = stats
        .contrast
        .iter()
        .filter_map(|p| {
            let (fg, bg) = (colour(&p.fg)?.solid(), colour(&p.bg)?.solid());
            let ratio = contrast(fg, bg);
            let need = if p.large { 3.0 } else { 4.5 };
            (ratio < need).then(|| Warning {
                fg,
                bg,
                ratio: (ratio * 100.0).round() / 100.0,
                level: if ratio < 3.0 { "fail" } else { "warning" },
                large: p.large,
                place: safe_name(&p.place, 100).unwrap_or_default(),
                weight: p.weight,
            })
        })
        .collect();
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    out.truncate(8);
    out
}
