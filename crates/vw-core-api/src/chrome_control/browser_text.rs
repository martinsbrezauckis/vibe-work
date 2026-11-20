//! Browser names in fixed agent refusals. Each producer names the refusal it emits with [`Fixed`] and words it for
//! the agent's browser; the wording comes from that id alone, never from matching an English sentence. `render`
//! remains for vocabulary texts stored in setup state and passes every other text through verbatim.
use super::agent_browser::Browser;

/// One fixed agent refusal whose wording names the agent's browser (`{browser}` is the short name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fixed {
    /// Tool arguments carry a reserved or internal field.
    InvalidArguments,
    /// `chrome_execute` without a session id of this chat.
    OpenSessionFirst,
    /// `client.selection`, which only the user sends from the browser's own menu.
    SelectionFromUser,
    /// The session id is known but belongs to another chat.
    OtherChat,
    /// This chat's session ended because the browser disconnected.
    LostConnection,
    /// This chat's session was stopped and is not retained.
    Stopped,
    /// This chat's session ended because the chat's access mode was lowered.
    ModeLowered,
    /// The action is not in the browser action catalog.
    UnknownAction,
    /// The browser package is not part of this installation.
    Unavailable,
}

impl Fixed {
    /// Every id, for tests that walk the full set.
    #[cfg(test)]
    pub(super) const ALL: [Fixed; 9] = [
        Fixed::InvalidArguments,
        Fixed::OpenSessionFirst,
        Fixed::SelectionFromUser,
        Fixed::OtherChat,
        Fixed::LostConnection,
        Fixed::Stopped,
        Fixed::ModeLowered,
        Fixed::UnknownAction,
        Fixed::Unavailable,
    ];

    fn template(self, browser: Browser) -> &'static str {
        match self {
            Self::InvalidArguments => {
                "Not run: invalid {browser} tool arguments. Use chrome_status for the action's argument schema."
            }
            Self::OpenSessionFirst => {
                "Not run: open {article} {browser} browser session in this chat first."
            }
            // Hidden Firefox page sharing (firefox_sharing::PAGE_SHARING_AVAILABLE) has no working Ask Vibe Work menu.
            Self::SelectionFromUser
                if browser == Browser::Firefox
                    && !super::firefox_sharing::PAGE_SHARING_AVAILABLE =>
            {
                "Not run: selection is sent by the user, and sharing pages from {browser} is not available in this version of Vibe Work. Use chrome_status for the owned {browser} window's supported actions."
            }
            Self::SelectionFromUser => {
                "Not run: selection is sent by the user from {browser}. Ask the user to use Ask Vibe Work in {browser}'s context menu."
            }
            Self::OtherChat => {
                "Not run: this {browser} browser session belongs to another chat. Open {article} {browser} browser session for this chat with chrome_session."
            }
            Self::LostConnection => {
                "Not run: this {browser} browser session lost its connection to {browser}. Check chrome_status, reconnect the browser if needed, then open a new browser session with chrome_session."
            }
            Self::Stopped => {
                "Not run: this {browser} browser session was stopped; open a new one with chrome_session."
            }
            Self::ModeLowered => {
                "Not run: this {browser} browser session ended because this chat's access mode was lowered; its tabs stay open. Open a new one with chrome_session if the new mode allows the work."
            }
            Self::UnknownAction => {
                "Not run: unknown {browser} action. Use chrome_status to find supported actions."
            }
            Self::Unavailable => UNAVAILABLE,
        }
    }

    /// The refusal in `browser`'s words.
    pub(crate) fn text(self, browser: Browser) -> String {
        browser.words(self.template(browser))
    }
}

/// Words vocabulary texts that setup state stores for Settings (the unavailable and broken-package labels) as the
/// agent instruction for `browser`. Any other text, including page and error text, is returned unchanged.
pub(super) fn render(browser: Browser, text: &str) -> String {
    use vw_vocabulary::texts;
    match text {
        texts::CHROME_UNAVAILABLE
        | texts::EDGE_UNAVAILABLE
        | texts::FIREFOX_UNAVAILABLE
        | texts::CHROME_PACKAGE_MISSING
        | texts::CHROME_PACKAGE_CHANGED => unavailable(browser, text),
        // The owner-identity texts are shared by every browser; each names its own.
        texts::CHROME_OWNER_STORE_FAILED
        | texts::CHROME_OWNER_STORE_UNAVAILABLE
        | texts::CHROME_OWNER_READ_FAILED => browser.words(text),
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

#[cfg(test)]
#[path = "browser_text_tests.rs"]
mod tests;
