//! The words of Chrome Control's asks: what a call does, why the card asks, what the agent is told, and the native
//! confirmation of a raised level. Names come from
//! `vocabulary.json` (group `chromeControl`), so Settings, the card, the dialog and the agent read the same words.
//! Depends on: vw-vocabulary, `vw_ask::chrome`. Called by: `check`, `commands`.
use serde_json::Value;
use vw_ask::chrome::{Group, Level};
use vw_ask::{ChromeSettings, CoversPart};

/// A group's or level's name ("Send forms and confirm", "Full: works on your behalf").
pub(super) fn name(id: &str) -> &'static str {
    vw_vocabulary::term("chromeControl", id).map_or("this action", |t| t.name)
}

/// What the tool `leaf` does, in plain words (never the operation's id).
fn action(leaf: &str) -> &'static str {
    match leaf.strip_prefix("chrome_").unwrap_or(leaf) {
        "page_read" => "Read the page",
        "page_wait" => "Wait for the page",
        "capture_screenshot" => "Take a screenshot",
        "tabs_inspect_other" => "Look at your open tabs",
        "page_navigate" => "Go to a site",
        "page_click" => "Click",
        "page_scroll" => "Scroll",
        "page_type" => "Type into a field",
        "page_key" => "Press a key",
        "tabs_create" => "Open a tab",
        "tabs_close_owned" => "Close a tab Vibe opened",
        "scope_add_origin" => "Add a site to this session",
        "page_interact" => "Fill in form fields",
        "files_download" => "Download a file",
        "files_upload" => "Upload a file you picked",
        "page_commit" => "Send a form or confirm",
        "auth_secrets" => "Sign in with a saved login",
        "cookies_metadata" => "See which cookies a site has",
        "cookies_change" => "Change cookies",
        "storage_metadata" => "See what a site stores",
        "storage_read" => "Read what a site stores",
        "storage_change" => "Change what a site stores",
        "raw_execute" => "Run a script on the page",
        "bookmarks_read" => "Read your bookmarks",
        "bookmarks_edit" => "Change your bookmarks",
        "history_read" => "Read your browsing history",
        "history_delete" => "Delete entries in your history",
        "tabs_close_user" => "Close one of your tabs",
        _ => "Use a Chrome Control action Vibe Work does not know yet",
    }
}

/// The site a call names, when its input has an address (`url`, `origin`): its host.
pub(super) fn site(input: &Value) -> Option<String> {
    ["url", "origin", "targetUrl"].iter().find_map(|key| {
        let text = input.get(*key)?.as_str()?;
        let rest = text.split_once("://").map_or(text, |(_, rest)| rest);
        let host = rest
            .split(['/', '?', '#'])
            .next()?
            .trim()
            .to_ascii_lowercase();
        (!host.is_empty()).then_some(host)
    })
}

/// The card's summary: the action, and the site when the call names one.
pub(super) fn summary(leaf: &str, input: &Value) -> String {
    match site(input) {
        Some(site) => format!("{} on {site}", action(leaf)),
        None => action(leaf).to_owned(),
    }
}

const PLACE: &str = "Settings › Browser › Your Chrome";

/// Why the card asks: the level and the level that includes the action; Bookmarks and History
/// ask at every level.
pub(super) fn reason(settings: ChromeSettings, group: Group) -> String {
    match group.level() {
        Some(needs) => format!(
            "Your level in {PLACE} is {}. {} is part of {}.",
            name(settings.level.id()),
            name(group.id()),
            name(needs.id())
        ),
        None => format!(
            "Vibe asks before it uses your {} at every level.",
            name(group.id()).to_lowercase()
        ),
    }
}

/// Why closing one of the user's tabs asks.
pub(super) fn close_tabs_reason() -> String {
    format!(
        "“{}” is off in {PLACE}, so Vibe asks first.",
        vw_vocabulary::label("browserPermissions", "closeUserTabs")
    )
}

/// Why a tool Vibe Work does not know asks.
pub(super) const UNKNOWN_REASON: &str = "Vibe Work does not know this Chrome Control action yet.";

/// What "Allow for this session" covers for `group` on `site`: exactly that group on that site (or, for a call that
/// names no site, only calls that name none either).
pub(super) fn covers(group: Group, site: Option<&str>) -> Vec<CoversPart> {
    let group = name(group.id());
    match site {
        Some(site) => vec![
            CoversPart::words(&format!("“{group}” on ")),
            CoversPart::code(site),
            CoversPart::words(" in this session"),
        ],
        None => vec![CoversPart::words(&format!(
            "“{group}” in this session, for calls that name no site"
        ))],
    }
}

/// What closing-tab session answers cover.
pub(super) fn close_tabs_covers() -> Vec<CoversPart> {
    vec![CoversPart::words("closing your tabs in this session")]
}

/// The agent's sentence after the user's No on the card (without the refusal code).
pub(super) fn said_no(summary: &str, note: Option<&str>) -> String {
    vw_ask::agent_text(
        &vw_ask::Outcome::Refused {
            by: vw_ask::AnsweredBy::Person,
            note: note.map(str::to_owned),
        },
        &format!("“{summary}” in their Chrome"),
    )
}

/// The native confirmation's text for raising the level `from` → `to`.
pub(super) fn raise_summary(from: ChromeSettings, to: ChromeSettings) -> String {
    let title = match to.level {
        Level::Act => "Let Vibe work on your behalf in your Chrome?",
        Level::Fill => "Let Vibe use Advanced in your Chrome?",
        Level::Browse => "Let Vibe use basic browser control in your Chrome?",
    };
    let added: Vec<String> = from
        .added(to)
        .iter()
        .map(|g| format!("• {}", name(g.id())))
        .collect();
    let mut text = format!(
        "{title}\n\n{}: {}\n\nIn tabs Vibe opened or you gave to a session, Vibe will also do these without asking \
         you first:\n{}",
        name(to.level.id()),
        vw_vocabulary::term("chromeControl", to.level.id()).map_or("", |t| t.description),
        added.join("\n")
    );
    if from.added(to).contains(&Group::SignIn) {
        text.push_str("\n\nA vault login is used only if you allowed it for the agent in Vault.");
    }
    text.push_str("\n\nYou can change this in ");
    text.push_str(PLACE);
    text.push('.');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_reason_names_the_level_and_the_level_that_includes_the_action() {
        let basic = ChromeSettings::default();
        let (send, full) = (name("send"), name("act"));
        assert_eq!(
            reason(basic, Group::Send),
            format!(
                "Your level in {PLACE} is {}. {send} is part of {full}.",
                name("browse")
            )
        );
        assert!(
            reason(ChromeSettings { level: Level::Act }, Group::History)
                .starts_with("Vibe asks before")
        );
    }

    #[test]
    fn the_summary_and_covers_name_the_site_without_operation_ids() {
        let input = json!({"url": "https://Shop.Example/cart?x=1"});
        assert_eq!(site(&input).as_deref(), Some("shop.example"));
        assert_eq!(
            summary("chrome_page_commit", &input),
            "Send a form or confirm on shop.example"
        );
        assert_eq!(
            summary("chrome_raw_execute", &json!({})),
            "Run a script on the page"
        );
        assert!(!summary("chrome_new_thing", &json!({})).contains("new_thing"));
        assert_eq!(
            covers(Group::Send, Some("shop.example"))[1],
            CoversPart::code("shop.example")
        );
    }

    #[test]
    fn the_raise_dialog_lists_what_the_new_level_adds() {
        let text = raise_summary(
            ChromeSettings::default(),
            ChromeSettings { level: Level::Act },
        );
        assert!(
            text.starts_with("Let Vibe work on your behalf in your Chrome?"),
            "{text}"
        );
        assert!(text.contains(&format!("• {}", name("send"))));
        assert!(text.contains("A vault login is used only if you allowed it"));
        assert!(text.ends_with(&format!("You can change this in {PLACE}.")));
    }
}

#[cfg(test)]
mod refusal_tests {
    #[test]
    fn chrome_no_uses_the_same_rule_and_preserves_the_note() {
        assert_eq!(
            super::said_no("Send", Some("save the draft")),
            "Not done: the user said No to “Send” in their Chrome. The user's note: save the draft"
        );
    }
}
