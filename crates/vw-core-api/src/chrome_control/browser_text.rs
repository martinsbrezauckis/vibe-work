//! Browser names for fixed legacy diagnostics. Called at the agent display boundary; dynamic text stays verbatim.
use super::agent_browser::Browser;

pub(super) fn render(browser: Browser, text: &str) -> String {
    use vw_vocabulary::texts;
    match text {
        texts::CHROME_UNAVAILABLE => browser.words("{yourBrowser} is not in this version of Vibe Work."),
        texts::CHROME_PACKAGE_MISSING => texts::BROWSER_PACKAGE_MISSING.replace("{browser}", browser.name()),
        texts::CHROME_PACKAGE_CHANGED => texts::BROWSER_PACKAGE_CHANGED.replace("{browser}", browser.name()),
        "Connect Chrome" => browser.words("Connect {browser}"),
        "Not run: Chrome is unavailable in this access mode." => browser.words("Not run: {browser} is unavailable in this access mode."),
        "Not run: invalid Chrome tool arguments." => browser.words("Not run: invalid {browser} tool arguments."),
        "Not run: no Chrome session with this id; open one with chrome_session." => browser.words("Not run: no {browser} session with this id; open one with chrome_session."),
        "Not run: open a Chrome session in this chat first." => browser.words("Not run: open a {browser} session in this chat first."),
        "Not run: selection is sent by the person from Chrome. Use Ask Vibe Work in Chrome's context menu." => browser.words("Not run: selection is sent by the person from {browser}. Use Ask Vibe Work in {browser}'s context menu."),
        "Not run: this Chrome session belongs to another chat." => browser.words("Not run: this {browser} session belongs to another chat."),
        "Not run: this Chrome session lost its connection to Chrome." => browser.words("Not run: this {browser} session lost its connection to {browser}."),
        "Not run: this Chrome session was stopped." => browser.words("Not run: this {browser} session was stopped."),
        "Not run: this Chrome session was stopped; open a new one with chrome_session." => browser.words("Not run: this {browser} session was stopped; open a new one with chrome_session."),
        "Not run: this chat's Chrome work ended." => browser.words("Not run: this chat's {browser} work ended."),
        "Not run: unknown Chrome action. Use chrome_status to find supported actions." => browser.words("Not run: unknown {browser} action. Use chrome_status to find supported actions."),
        _ => text.to_owned(),
    }
}
