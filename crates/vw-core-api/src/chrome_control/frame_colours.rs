//! Each chat's agent control frame colour, kept with the app's other per-chat records (`chat-models.json` sits in
//! the same folder; the built-in browser frames chats without any helper, so not in the helper's state folder) so a
//! chat's frame has the same colour after a restart and in every browser that frames its work (the built-in
//! browser, Chrome, Edge, Firefox). One colour for every chat is the default (index 0); a chat keeps
//! another index only once it was set (the per-chat colour setting, through the `chromeControl` command).
//!
//! Depends on: `profile_storage` (bounded regular-file read), `vw_platform::replace_private`. Called by: `frames`
//! (the index sent with `control.frame`), the `ChromeControl` view (the built-in frame) and the setting command.
use std::collections::BTreeMap;
use std::path::Path;

/// File name in the chats' records folder.
const FILE: &str = "chat-frame-colours.json";
/// The palette size every frame shares (the helper accepts the same indexes; anything else shows index 0).
pub(crate) const PALETTE: u8 = 8;
const LIMIT: u64 = 1024 * 1024;

/// Chat id -> palette index; chats at the default index are not stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Colours(BTreeMap<String, u8>);

impl Colours {
    /// Reads the saved colours; a missing file is the default, an unreadable one too (a colour is never authority).
    pub(crate) fn load(dir: Option<&Path>) -> Self {
        let saved: BTreeMap<String, u8> =
            super::profile_storage::load(dir, FILE, LIMIT).unwrap_or_default();
        Self(
            saved
                .into_iter()
                .filter(|(_, index)| *index < PALETTE && *index != 0)
                .collect(),
        )
    }

    /// The chat's index (0 when none is stored).
    pub(crate) fn get(&self, chat: &str) -> u8 {
        self.0.get(chat).copied().unwrap_or(0)
    }

    /// Every stored chat colour.
    pub(crate) fn all(&self) -> BTreeMap<String, u8> {
        self.0.clone()
    }

    /// Sets (or, for 0, clears) a chat's index. Answers whether anything changed.
    pub(crate) fn set(&mut self, chat: &str, index: u8) -> bool {
        let before = self.get(chat);
        if index == 0 {
            self.0.remove(chat);
        } else {
            self.0.insert(chat.to_owned(), index);
        }
        before != index
    }

    /// Saves the colours (no folder: kept in memory only).
    ///
    /// # Errors
    /// The person-facing reason when the file could not be written.
    pub(crate) fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = dir else { return Ok(()) };
        let failed = || "The chat's frame colour could not be saved.".to_owned();
        std::fs::create_dir_all(dir).map_err(|_| failed())?;
        let bytes = serde_json::to_vec(&self.0).map_err(|_| failed())?;
        vw_platform::replace_private(&dir.join(FILE), &bytes).map_err(|_| failed())
    }
}

#[cfg(test)]
mod tests {
    //! PREDICT (written before the file): a chat's colour lives only in memory, so a restart shows every chat at
    //! index 0 again.
    use super::{Colours, PALETTE};

    #[test]
    fn a_chat_keeps_its_colour_across_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut colours = Colours::load(Some(dir.path()));
        assert_eq!(
            colours.get("chat-a"),
            0,
            "one colour for every chat by default"
        );
        assert!(colours.set("chat-a", 3));
        assert!(!colours.set("chat-a", 3));
        colours.save(Some(dir.path())).unwrap();
        let reloaded = Colours::load(Some(dir.path()));
        assert_eq!(reloaded.get("chat-a"), 3);
        assert_eq!(reloaded.get("chat-b"), 0);
    }

    #[test]
    fn unknown_indexes_and_unreadable_files_fall_back_to_the_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("chat-frame-colours.json"),
            format!("{{\"chat-a\":{PALETTE},\"chat-b\":2}}"),
        )
        .unwrap();
        let loaded = Colours::load(Some(dir.path()));
        assert_eq!((loaded.get("chat-a"), loaded.get("chat-b")), (0, 2));
        std::fs::write(dir.path().join("chat-frame-colours.json"), b"{broken").unwrap();
        assert_eq!(Colours::load(Some(dir.path())), Colours::default());
    }

    #[test]
    fn setting_the_default_clears_the_stored_entry() {
        let mut colours = Colours::default();
        colours.set("chat-a", 5);
        assert!(colours.set("chat-a", 0));
        assert!(colours.all().is_empty());
    }
}
