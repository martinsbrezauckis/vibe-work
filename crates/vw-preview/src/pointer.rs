//! Shared native click semantics and expected DOM evidence. Browser actions build this; engine adapters consume it.
//! Depends on: serde value types.
//! Called by: browser pointer parsing and native engine adapters.
use serde::{Deserialize, Serialize};

/// One bounded native pointer sequence, using DOM button ids and `DevTools` modifier bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClickOptions {
    /// DOM button: 0 left, 1 middle, 2 right.
    pub button: u8,
    /// Number of consecutive clicks, one through three; zero only for [`Self::HOVER`].
    pub count: u8,
    /// Alt=1, Control=2, Meta=4, Shift=8.
    pub modifiers: u8,
    /// Optional horizontal and vertical pixel deltas; wheel input holds no button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel: Option<(i32, i32)>,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            button: 0,
            count: 1,
            modifiers: 0,
            wheel: None,
        }
    }
}

impl ClickOptions {
    /// Move the engine pointer without pressing a button. Only host hover actions construct this.
    pub const HOVER: Self = Self {
        button: 0,
        count: 0,
        modifiers: 0,
        wheel: None,
    };

    /// Refuse malformed host input before reaching an engine.
    #[must_use]
    pub fn valid(self) -> bool {
        if let Some((x, y)) = self.wheel {
            return self.button == 0
                && self.count == 0
                && self.modifiers == 0
                && (-20_000..=20_000).contains(&x)
                && (-20_000..=20_000).contains(&y)
                && (x != 0 || y != 0);
        }
        self == Self::HOVER
            || self.button <= 2
                && (1..=3).contains(&self.count)
                && self.modifiers <= 15
                && (self.button == 0 || self.count == 1)
    }

    /// The ordinary mouse events required to prove this sequence; context-menu timing differs between engines.
    #[must_use]
    pub fn events(self) -> Vec<&'static str> {
        if self.wheel.is_some() {
            return vec!["wheel"];
        }
        if self == Self::HOVER {
            return vec!["mousemove"];
        }
        let mut events = Vec::new();
        for count in 1..=self.count {
            events.extend(["mousedown", "mouseup"]);
            if self.button == 0 {
                events.push("click");
                if count == 2 {
                    events.push("dblclick");
                }
            }
        }
        events
    }
}
