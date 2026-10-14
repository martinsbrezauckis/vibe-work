//! Colour roles, the type scale and component tokens of the design capture; split from `design` for size.
//!
//! Colour roles (names from the DESIGN.md format's recommended set: `primary`, `secondary`, `neutral`, `surface`,
//! `on-surface`, `error`, plus `muted` and `border`):
//! - `neutral`: the page background (body, else root, else the largest painted area).
//! - `on-surface`: the colour of most text.
//! - `primary`: the strongest brand signal: a root custom property named primary / brand / accent / cta, filled
//!   buttons, link colours, then chromatic colours by use (Canvas's role words, re-weighted by use).
//! - `secondary`: the next chromatic candidate clearly different from `primary`.
//! - `surface`: the second most painted background (cards, panels), `muted`: a grey text colour with real use,
//!   `border`: the most drawn border colour, `error`: a custom property named danger / error / destructive.
//!
//! Depends on: `design` (types), `colour`. Called by: `design::derive`.

use std::collections::BTreeMap;

use super::colour::{Rgba, contrast};
use super::design::{ColourToken, Stats, Tokens, TypeToken, px, safe_name, trim_number};

fn colour(text: &str) -> Option<Rgba> {
    Rgba::parse(text)
}

fn token(name: &str, value: Rgba, why: impl Into<String>) -> ColourToken {
    ColourToken {
        name: name.to_owned(),
        value,
        why: why.into(),
    }
}

/// Fills `tokens.colours`, `palette`, `colour_vars`, `theme`, `monochrome`.
pub(super) fn colours(stats: &Stats, tokens: &mut Tokens) {
    let background = stats
        .page
        .background
        .as_deref()
        .and_then(colour)
        .filter(|c| c.opaque())
        .or_else(|| {
            stats
                .backgrounds
                .iter()
                .filter_map(|r| colour(&r.value))
                .find(|c| c.opaque())
        })
        .unwrap_or(Rgba {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        });
    tokens.theme = if background.luminance() < 0.2 {
        "dark"
    } else {
        "light"
    };
    let text = stats
        .text
        .iter()
        .filter_map(|r| colour(&r.value).map(|c| (c.solid(), r)))
        .find(|(c, _)| contrast(*c, background) >= 2.0)
        .map(|(c, _)| c);

    let (ranked, error) = ranked(stats);
    let text_total: f64 = stats.text.iter().map(|r| r.weight).sum::<f64>().max(1.0);
    tokens.monochrome = !ranked.iter().any(|(c, _, _)| c.chromatic());
    let primary = ranked
        .first()
        .map(|(c, _, why)| (*c, why.join(", ")))
        .or_else(|| text.map(|t| (t, "no brand colour: the text colour leads".to_owned())));
    if let Some((c, why)) = &primary {
        tokens
            .colours
            .push(token("primary", *c, format!("used for {why}")));
    }
    if let Some((c, _, why)) = ranked
        .iter()
        .skip(1)
        .find(|(c, _, _)| primary.as_ref().is_none_or(|(p, _)| c.distance(*p) > 60.0))
    {
        tokens.colours.push(token(
            "secondary",
            *c,
            format!("used for {}", why.join(", ")),
        ));
    }
    tokens
        .colours
        .push(token("neutral", background, "the page background (body)"));
    if let Some(c) = stats
        .backgrounds
        .iter()
        .filter_map(|r| colour(&r.value))
        .filter(|c| c.opaque() && c.distance(background) > 6.0)
        .find(|c| !c.chromatic())
    {
        tokens.colours.push(token(
            "surface",
            c,
            "the next most painted background (cards, panels)",
        ));
    }
    if let Some(t) = text {
        tokens
            .colours
            .push(token("on-surface", t, "the colour of most text"));
    }
    text_roles(stats, tokens, text, background, text_total);
    if let Some(c) = error {
        tokens
            .colours
            .push(token("error", c, "a danger/error style variable"));
    }
    palette(stats, tokens);
}

/// `muted` (a grey text colour with real use, weaker than the main text) and `border`.
fn text_roles(
    stats: &Stats,
    tokens: &mut Tokens,
    text: Option<Rgba>,
    background: Rgba,
    text_total: f64,
) {
    if let Some(t) = text {
        let muted = stats
            .text
            .iter()
            .filter(|r| r.weight / text_total >= 0.02)
            .filter_map(|r| colour(&r.value))
            .map(Rgba::solid)
            .find(|c| {
                !c.chromatic()
                    && c.distance(t) > 20.0
                    && contrast(*c, background) < contrast(t, background)
            });
        if let Some(c) = muted {
            tokens
                .colours
                .push(token("muted", c, "secondary text (captions, meta lines)"));
        }
    }
    if let Some(c) = stats.borders.iter().find_map(|r| colour(&r.value)) {
        tokens
            .colours
            .push(token("border", c, "the most drawn border colour"));
    }
}

/// The palette by use (text + painted areas + borders, each normalised) and the colour custom properties.
fn palette(stats: &Stats, tokens: &mut Tokens) {
    let mut uses: BTreeMap<String, (Rgba, [f64; 3])> = BTreeMap::new();
    for (i, rows) in [&stats.text, &stats.backgrounds, &stats.borders]
        .into_iter()
        .enumerate()
    {
        let total: f64 = rows.iter().map(|r| r.weight).sum::<f64>().max(1.0);
        for row in rows {
            if let Some(c) = colour(&row.value) {
                let entry = uses.entry(c.hex()).or_insert((c, [0.0; 3]));
                entry.1[i] += row.weight / total;
            }
        }
    }
    let mut list: Vec<(Rgba, [f64; 3])> = uses.into_values().collect();
    let top = |s: &[f64; 3]| s.iter().copied().fold(0.0, f64::max);
    list.sort_by(|a, b| top(&b.1).total_cmp(&top(&a.1)));
    list.truncate(16);
    tokens.palette = list;
    tokens.colour_vars = stats
        .vars
        .iter()
        .filter_map(|v| {
            let c = v.colour.as_deref().and_then(colour)?;
            let name = safe_name(v.name.trim_start_matches('-'), 60)?;
            let name: String = name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                        ch.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            (!name.is_empty()).then_some((name, c))
        })
        .take(40)
        .collect();
}

/// The family stack as safe names (quotes dropped).
fn stack(family: &str) -> Vec<String> {
    family
        .split(',')
        .filter_map(|f| safe_name(f.trim().trim_matches(['"', '\'']), 60))
        .take(6)
        .collect()
}

fn type_token(name: &str, row: &super::design::FontRow) -> Option<TypeToken> {
    let size = px(&row.size)?;
    let families = stack(&row.family);
    let family = families.first()?.clone();
    let line_height = px(&row.line_height)
        .filter(|_| row.line_height != "normal")
        .map(|lh| (lh / size * 100.0).round() / 100.0);
    let letter_spacing = px(&row.letter_spacing).filter(|v| v.abs() >= 0.05);
    Some(TypeToken {
        name: name.to_owned(),
        family,
        stack: families,
        size,
        weight: row.weight.trim().parse().unwrap_or(400),
        line_height,
        letter_spacing,
        uppercase: row.transform == "uppercase",
    })
}

/// Headline levels from heading tags (largest first), the body size with the most text, a larger and a smaller body
/// size with real use, and the label style of buttons and form labels.
pub(super) fn typography(stats: &Stats, tokens: &mut Tokens) {
    let total: f64 = stats.fonts.iter().map(|f| f.chars).sum::<f64>().max(1.0);
    let heading = |f: &&super::design::FontRow| {
        f.tags
            .iter()
            .any(|t| t.len() == 2 && t.starts_with('h') && t != "hr")
    };
    let mut headings: Vec<&super::design::FontRow> = stats.fonts.iter().filter(heading).collect();
    headings.sort_by(|a, b| {
        px(&b.size)
            .unwrap_or(0.0)
            .total_cmp(&px(&a.size).unwrap_or(0.0))
    });
    let mut sizes_taken: Vec<f64> = Vec::new();
    let mut levels = Vec::new();
    for row in headings {
        let Some(size) = px(&row.size) else { continue };
        if sizes_taken.iter().any(|s| (s - size).abs() < 1.0) {
            continue;
        }
        sizes_taken.push(size);
        levels.push(row);
        if levels.len() >= 4 {
            break;
        }
    }
    let names: &[&str] = if levels
        .first()
        .and_then(|r| px(&r.size))
        .is_some_and(|s| s >= 40.0)
    {
        &[
            "headline-display",
            "headline-lg",
            "headline-md",
            "headline-sm",
        ]
    } else {
        &["headline-lg", "headline-md", "headline-sm", "headline-xs"]
    };
    for (row, name) in levels.iter().zip(names) {
        if let Some(t) = type_token(name, row) {
            tokens.typography.push(t);
        }
    }
    let body_like = |f: &&super::design::FontRow| !heading(f);
    let body = stats
        .fonts
        .iter()
        .filter(body_like)
        .max_by(|a, b| a.chars.total_cmp(&b.chars));
    if let Some(body) = body {
        let body_size = px(&body.size).unwrap_or(16.0);
        if let Some(t) = type_token("body-md", body) {
            tokens.typography.push(t);
        }
        let used = |f: &&super::design::FontRow| f.chars / total >= 0.03;
        let larger = stats
            .fonts
            .iter()
            .filter(body_like)
            .filter(used)
            .filter(|f| px(&f.size).is_some_and(|s| s > body_size + 0.5))
            .max_by(|a, b| a.chars.total_cmp(&b.chars));
        if let Some(t) = larger.and_then(|r| type_token("body-lg", r)) {
            tokens.typography.push(t);
        }
        let smaller = stats
            .fonts
            .iter()
            .filter(body_like)
            .filter(used)
            .filter(|f| px(&f.size).is_some_and(|s| s < body_size - 0.5))
            .max_by(|a, b| a.chars.total_cmp(&b.chars));
        if let Some(t) = smaller.and_then(|r| type_token("body-sm", r)) {
            tokens.typography.push(t);
        }
    }
    // Buttons first: form labels often share the body style (matching by label can pick the body row through "label").
    let tagged =
        |tag: &'static str| move |f: &&super::design::FontRow| f.tags.iter().any(|t| t == tag);
    let label = stats
        .fonts
        .iter()
        .filter(tagged("button"))
        .max_by(|a, b| a.chars.total_cmp(&b.chars))
        .or_else(|| {
            stats
                .fonts
                .iter()
                .filter(tagged("label"))
                .max_by(|a, b| a.chars.total_cmp(&b.chars))
        });
    if let Some(t) = label.and_then(|r| type_token("label-md", r)) {
        tokens.typography.push(t);
    }
}

/// A colour as a reference to its token when one has it (`{colors.primary}`), else its hex.
fn colour_ref(tokens: &Tokens, value: Option<&str>) -> Option<String> {
    let c = colour(value?)?;
    Some(
        tokens
            .colours
            .iter()
            .find(|t| t.value == c.solid() && c.opaque())
            .map_or_else(|| c.hex(), |t| format!("{{colors.{}}}", t.name)),
    )
}

fn rounded_ref(tokens: &Tokens, value: &str) -> String {
    let v = px(value).unwrap_or(0.0);
    if v >= 999.0 || value.ends_with('%') {
        return "{rounded.full}".to_owned();
    }
    tokens
        .rounded
        .iter()
        .find(|(_, r)| px(r).is_some_and(|p| (p - v).abs() < 0.5))
        .map_or_else(
            || format!("{}px", trim_number(v)),
            |(n, _)| format!("{{rounded.{n}}}"),
        )
}

/// Component tokens: the two most common buttons (a filled one is `button-primary`), the input, the card, the link.
pub(super) fn components(stats: &Stats, tokens: &mut Tokens) {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let plain = |v: &str, max: usize| safe_name(v, max).unwrap_or_default();
    let mut buttons: Vec<&super::design::Component> = stats
        .components
        .get("button")
        .into_iter()
        .flatten()
        .collect();
    buttons.sort_by_key(|b| {
        u8::from(
            !b.background_color
                .as_deref()
                .and_then(colour)
                .is_some_and(Rgba::chromatic),
        )
    });
    // A second button style counts only when it is used more than once (one unstyled button is not a variant).
    let second_ok = buttons.get(1).is_some_and(|b| b.count >= 2);
    for (i, b) in buttons
        .iter()
        .take(if second_ok { 2 } else { 1 })
        .enumerate()
    {
        let name = if i == 0 {
            "button-primary"
        } else {
            "button-secondary"
        };
        let mut props = Vec::new();
        if let Some(bg) = colour_ref(tokens, b.background_color.as_deref()) {
            props.push(("backgroundColor".to_owned(), bg));
        }
        if let Some(fg) = colour_ref(tokens, b.text_color.as_deref()) {
            props.push(("textColor".to_owned(), fg));
        }
        props.push(("rounded".to_owned(), rounded_ref(tokens, &b.rounded)));
        props.push(("padding".to_owned(), plain(&b.padding, 30)));
        props.push(("height".to_owned(), plain(&b.height, 12)));
        out.push((name.to_owned(), props));
    }
    for (kind, name) in [("input", "input-field"), ("card", "card")] {
        if let Some(c) = stats.components.get(kind).and_then(|v| v.first()) {
            let mut props = Vec::new();
            if let Some(bg) = colour_ref(tokens, c.background_color.as_deref()) {
                props.push(("backgroundColor".to_owned(), bg));
            }
            if let Some(fg) = colour_ref(tokens, c.text_color.as_deref()) {
                props.push(("textColor".to_owned(), fg));
            }
            props.push(("rounded".to_owned(), rounded_ref(tokens, &c.rounded)));
            props.push(("padding".to_owned(), plain(&c.padding, 30)));
            if kind == "input" {
                props.push(("height".to_owned(), plain(&c.height, 12)));
            }
            out.push((name.to_owned(), props));
        }
    }
    if let Some(link) = stats.components.get("link").and_then(|v| v.first())
        && let Some(fg) = colour_ref(tokens, link.text_color.as_deref())
    {
        out.push(("link".to_owned(), vec![("textColor".to_owned(), fg)]));
    }
    tokens.components = out;
}

/// Brand-colour candidates, strongest first (score, and what they were used for), and the error colour of a
/// danger/error style variable.
#[allow(clippy::type_complexity)]
fn ranked(stats: &Stats) -> (Vec<(Rgba, f64, Vec<&'static str>)>, Option<Rgba>) {
    let mut candidates: BTreeMap<String, (Rgba, f64, Vec<&'static str>)> = BTreeMap::new();
    let mut add = |c: Rgba, score: f64, why: &'static str| {
        let c = c.solid();
        let entry = candidates.entry(c.hex()).or_insert((c, 0.0, Vec::new()));
        entry.1 += score;
        if !entry.2.contains(&why) {
            entry.2.push(why);
        }
    };
    let mut error = None;
    for var in &stats.vars {
        let Some(c) = var.colour.as_deref().and_then(colour) else {
            continue;
        };
        let name = var.name.to_ascii_lowercase();
        if name.contains("primary") {
            add(c, 120.0, "the page's --primary style variable");
        } else if ["brand", "accent", "cta"].iter().any(|w| name.contains(w)) {
            add(c, 90.0, "a brand/accent style variable");
        }
        if error.is_none()
            && ["danger", "error", "destructive"]
                .iter()
                .any(|w| name.contains(w))
        {
            error = Some(c.solid());
        }
    }
    for button in stats.components.get("button").into_iter().flatten() {
        if let Some(c) = button.background_color.as_deref().and_then(colour)
            && c.chromatic()
        {
            #[allow(clippy::cast_precision_loss)]
            add(c, 40.0 * button.count.min(20) as f64, "filled buttons");
        }
    }
    let link_total: f64 = stats.links.iter().map(|r| r.weight).sum::<f64>().max(1.0);
    for row in &stats.links {
        if let Some(c) = colour(&row.value).filter(|c| c.chromatic()) {
            add(c, 30.0 + 40.0 * row.weight / link_total, "links");
        }
    }
    let text_total: f64 = stats.text.iter().map(|r| r.weight).sum::<f64>().max(1.0);
    let area_total: f64 = stats
        .backgrounds
        .iter()
        .map(|r| r.weight)
        .sum::<f64>()
        .max(1.0);
    for row in &stats.text {
        if let Some(c) = colour(&row.value).filter(|c| c.chromatic()) {
            add(c, 25.0 * row.weight / text_total, "text");
        }
    }
    for row in &stats.backgrounds {
        if let Some(c) = colour(&row.value).filter(|c| c.chromatic() && c.opaque()) {
            add(c, 25.0 * row.weight / area_total, "painted areas");
        }
    }
    let mut ranked: Vec<(Rgba, f64, Vec<&'static str>)> = candidates.into_values().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    (ranked, error)
}
