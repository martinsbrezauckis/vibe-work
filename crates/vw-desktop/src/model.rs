//! Bounded accessibility observations and explicit action postconditions.
//! Depends on serde and AT-SPI types; used by the engine, bus adapter and Rust callers.

use atspi::zbus::{names::OwnedUniqueName, zvariant::OwnedObjectPath};
use atspi::{Role, State, StateSet};
use serde::{Deserialize, Serialize};

/// Maximum number of elements in a snapshot (also caps window discovery).
pub const MAX_ELEMENTS: usize = 256;
/// Maximum descendant depth in a snapshot; the window has depth zero.
pub const MAX_DEPTH: u16 = 32;
/// Maximum UTF-8 bytes in each returned accessible name.
pub const MAX_NAME_BYTES: usize = 512;
/// Maximum combined bytes of an element's bus name and object path.
pub const MAX_ID_BYTES: usize = 1024;
/// Maximum UTF-8 bytes in requested or returned text.
pub const MAX_TEXT_BYTES: usize = 4096;

/// Stable identity while the provider keeps this accessible object alive.
/// Uses the unique D-Bus connection name and provider's object path, never a child index or label.
/// D-Bus name/path parsing and serde validation use zbus's standard types.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementId {
    /// Provider's unique name on this accessibility bus; not a well-known service name.
    pub bus: OwnedUniqueName,
    /// Provider's accessible object path.
    pub path: OwnedObjectPath,
}

/// Screen-coordinate rectangle supplied by the Component interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    /// Left edge in pixels.
    pub x: i32,
    /// Top edge in pixels.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

/// One compact accessibility tree entry. Names are untrusted application data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    /// Provider identity, unchanged by renaming or sibling reordering.
    pub id: ElementId,
    /// Standard AT-SPI role.
    pub role: Role,
    /// Accessible name, capped at [`MAX_NAME_BYTES`] on a UTF-8 boundary.
    pub name: String,
    /// True when the name is only a prefix and cannot prove an exact-name postcondition.
    pub name_truncated: bool,
    /// Standard AT-SPI states.
    pub state: StateSet,
    /// Component extents, or None when the provider has no Component interface.
    pub bounds: Option<Bounds>,
    /// Depth in the selected window; direct element reads use zero.
    pub depth: u16,
}

/// Preorder list of the selected window and its descendants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Selected window's identity.
    pub window: ElementId,
    /// At most [`MAX_ELEMENTS`] entries.
    pub elements: Vec<Element>,
    /// True if a size or depth limit omitted descendants.
    pub truncated: bool,
}

/// Observable state to prove, rather than assuming a successful method reply proves an effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Postcondition {
    /// Exact complete accessible name (for example a button's changed label).
    Name(String),
    /// Exact complete Text contents.
    Text(String),
    /// Presence or absence of a standard AT-SPI state.
    State {
        /// State to inspect.
        state: State,
        /// Required presence.
        present: bool,
    },
}

/// Exactly one AT-SPI mutation, followed by a read-only verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Invoke Action index zero, the AT-SPI default press/click action.
    Press {
        /// The caller specifies the observable effect on this same element.
        expected: Postcondition,
    },
    /// Replace `EditableText` contents; verify the exact text through Text.
    SetText(String),
    /// Grab Component focus; verify `State::Focused`.
    Focus,
}

/// A capped text read; a prefix never proves exact equality.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextValue {
    /// Observed text, capped at [`MAX_TEXT_BYTES`] on a UTF-8 boundary.
    pub value: String,
    /// True when the observed text is incomplete.
    pub truncated: bool,
}

/// Evidence from fresh reads of the target after dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// True only if the requested state holds in these fresh observations.
    pub matched: bool,
    /// Fresh accessible properties and state.
    pub observed: Element,
    /// Fresh Text contents, only for a text postcondition.
    pub text: Option<TextValue>,
}

/// Method acceptance and independently observed effect of one action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The provider's mutation method returned true; this alone is not success.
    pub accepted: bool,
    /// Fresh postcondition evidence, including an unmet state when applicable.
    pub verification: Verification,
}

impl Receipt {
    /// Success requires both method acceptance and a matching fresh observation.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.accepted && self.verification.matched
    }
}

/// Explicit pilot failures; none can be interpreted as a successful action.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Session/accessibility bus connection or protocol failure.
    #[error("AT-SPI connection failed: {0}")]
    Atspi(#[from] atspi::AtspiError),
    /// A provider call failed, including a stale or vanished object.
    #[error("AT-SPI provider call failed: {0}")]
    Bus(#[from] atspi::zbus::Error),
    /// No window satisfied the selector.
    #[error("No matching accessible window")]
    WindowNotFound,
    /// More than one window satisfied the selector.
    #[error("More than one accessible window matches; select a unique window name")]
    AmbiguousWindow,
    /// A complete window selection could not be made within the discovery cap.
    #[error("Window discovery exceeds the {MAX_ELEMENTS}-element limit")]
    DiscoveryLimit,
    /// A provider returned a null accessible reference.
    #[error("The accessible element is unavailable")]
    MissingElement,
    /// The needed AT-SPI interface/action is not supported.
    #[error("The accessible element does not support {0}")]
    Unsupported(&'static str),
    /// Provider returned a malformed count or reference.
    #[error("The AT-SPI provider returned an invalid {0}")]
    InvalidReply(&'static str),
    /// The live element cannot accept this requested action.
    #[error("The accessible element cannot accept this action in its current state")]
    NotActionable,
    /// Input or identity exceeds the pilot's response bounds.
    #[error("The {0} exceeds the pilot's size limit")]
    SizeLimit(&'static str),
}

pub(crate) fn cap(value: &mut String, limit: usize) -> bool {
    if value.len() <= limit {
        return false;
    }
    let end = value
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= limit)
        .last()
        .unwrap_or(0);
    value.truncate(end);
    true
}
