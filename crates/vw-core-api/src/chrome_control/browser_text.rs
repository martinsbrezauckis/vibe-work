//! Browser names for fixed legacy diagnostics. Called at the agent display boundary; dynamic text stays verbatim.
use super::agent_browser::Browser;

pub(super) fn render(browser: Browser, text: &str) -> String {
    use vw_vocabulary::texts;
    match text {
        texts::CHROME_UNAVAILABLE | texts::EDGE_UNAVAILABLE | texts::FIREFOX_UNAVAILABLE
        | texts::CHROME_PACKAGE_MISSING | texts::CHROME_PACKAGE_CHANGED => unavailable(browser, text),
        "Connect Chrome" => browser.words("Connect {browser}"),
        "Not run: invalid Chrome tool arguments." => browser.words("Not run: invalid {browser} tool arguments."),
        "Not run: open a Chrome session in this chat first." => browser.words("Not run: open a {browser} session in this chat first."),
        // Hidden Firefox page sharing (firefox_sharing::PAGE_SHARING_AVAILABLE) has no working Ask Vibe Work menu.
        "Not run: selection is sent by the user from Chrome. Ask the user to use Ask Vibe Work in Chrome's context menu." if browser == Browser::Firefox && !super::firefox_sharing::PAGE_SHARING_AVAILABLE => browser.words("Not run: selection is sent by the user, and sharing pages from {browser} is not available in this version of Vibe Work. Use chrome_status for the owned {browser} window's supported actions."),
        "Not run: selection is sent by the user from Chrome. Ask the user to use Ask Vibe Work in Chrome's context menu." => browser.words("Not run: selection is sent by the user from {browser}. Ask the user to use Ask Vibe Work in {browser}'s context menu."),
        "Not run: this Chrome session belongs to another chat." => browser.words("Not run: this {browser} session belongs to another chat."),
        "Not run: this Chrome session lost its connection to Chrome." => browser.words("Not run: this {browser} session lost its connection to {browser}."),
        "Not run: this Chrome session was stopped." => browser.words("Not run: this {browser} session was stopped."),
        "Not run: this Chrome session was stopped; open a new one with chrome_session." => browser.words("Not run: this {browser} session was stopped; open a new one with chrome_session."),
        "Not run: unknown Chrome action. Use chrome_status to find supported actions." => browser.words("Not run: unknown {browser} action. Use chrome_status to find supported actions."),
        _ => text.to_owned(),
    }
}

/// Agent instructions for an unavailable browser; UI labels never stand in for the next step.
pub(super) const UNAVAILABLE: &str = "{yourBrowser} is not available in this version of Vibe Work. Use an available browser, or ask the user to install a version that includes it.";

pub(super) fn unavailable(browser: Browser, package_text: &str) -> String {
    use vw_vocabulary::texts;
    let template = if package_text == texts::CHROME_PACKAGE_MISSING
        || package_text == label(browser, texts::CHROME_PACKAGE_MISSING)
    {
        "This installation is missing a file {yourBrowser} needs. Ask the user to reinstall Vibe Work."
    } else if package_text == texts::CHROME_PACKAGE_CHANGED
        || package_text == label(browser, texts::CHROME_PACKAGE_CHANGED)
    {
        "A file {yourBrowser} needs differs from the one shipped with Vibe Work. Ask the user to reinstall Vibe Work."
    } else {
        UNAVAILABLE
    };
    browser.words(template)
}

/// Existing Settings labels retain their wording independently of agent instructions.
pub(super) fn label(browser: Browser, text: &str) -> String {
    use vw_vocabulary::texts;
    match text {
        texts::CHROME_UNAVAILABLE => {
            browser.words("{yourBrowser} is not in this version of Vibe Work.")
        }
        texts::CHROME_PACKAGE_MISSING => {
            texts::BROWSER_PACKAGE_MISSING.replace("{browser}", browser.name())
        }
        texts::CHROME_PACKAGE_CHANGED => {
            texts::BROWSER_PACKAGE_CHANGED.replace("{browser}", browser.name())
        }
        _ => text.to_owned(),
    }
}

/// Agent instruction for the shared per-browser availability switch.
pub(super) const SWITCHED_OFF: &str = "{yourBrowser} is switched off in Vibe Work. Ask the user to turn it on in Settings > Browser > {yourBrowser}.";
