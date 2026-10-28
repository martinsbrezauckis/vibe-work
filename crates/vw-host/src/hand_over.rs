//! "Send to Vibe Work": the files, folders and text another program gives the app, from the launch arguments
//! to the window. A system menu entry starts `vibe-work --attach <paths>` (text: `--attach-text -`, read from
//! standard input); a running app receives the same hand-over through its single-instance channel
//! ([`crate::instance`]), a starting app takes it from its own arguments; the inbox holds it until the window takes
//! it and places chips in the composer.
//!
//! Depends on: serde. Called by: `instance` (the wire form), `app` (arguments, the inbox, the take command),
//! [`crate::Host::take_hand_overs`].
//! Invariants:
//! - A hand-over only adds chips the user sees: there is no field that sends, types into the box or picks a chat
//!   (unknown fields are refused), and nothing here reaches Vibe.
//! - Text never travels in the arguments: only `--attach-text -` (standard input) is accepted.
//! - Paths are absolute and exist when admitted; the running app checks again ([`HandOver::admit`]), so a
//!   hand-over from any local program names only real files and folders. A path that does not exist is refused
//!   with its name, and the window shows that.
//! - Sizes are bounded: [`MAX_ITEMS`] paths and [`MAX_TEXT_BYTES`] of text per hand-over, [`MAX_PENDING`]
//!   waiting hand-overs, [`MAX_WIRE_BYTES`] on the channel.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// Most paths one hand-over adds; the rest are refused with one line.
pub const MAX_ITEMS: usize = 100;
/// Most bytes of text one hand-over carries (1 MiB).
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Most hand-overs waiting for the window; later ones are dropped (and logged) until it takes them.
pub const MAX_PENDING: usize = 32;
/// Largest hand-over the single-instance channel reads.
pub const MAX_WIRE_BYTES: usize = 4 << 20;

/// What another program handed over.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandOver {
    /// Absolute paths of files and folders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
    /// Text (a selection), when one was handed over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<HandOverText>,
    /// What was named but not admitted, with the reason; the window shows these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<HandOverRefusal>,
}

/// Handed-over text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandOverText {
    /// The text itself.
    pub text: String,
    /// The program it was selected in, when known (the macOS text service names it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Something named in a hand-over that was not admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandOverRefusal {
    /// The path or "the text", as the user would recognise it.
    pub name: String,
    /// Why, in plain words.
    pub reason: String,
}

impl HandOver {
    /// Whether there is nothing to show (a plain launch).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.text.is_none() && self.refused.is_empty()
    }

    /// Reads a hand-over from launch arguments (the program name included or not); `None` for a launch
    /// without `--attach` / `--attach-text`. Relative paths are resolved against `cwd` (a file manager script
    /// runs in the folder it shows). Exact `--attach` makes every remaining argument a literal path;
    /// file launchers put paths last. `stdin` is read only for the separate `--attach-text -` invocation.
    pub fn from_args<I>(args: I, cwd: &Path, stdin: impl Read) -> Option<Self>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut named = false;
        let mut capture = false;
        let mut paths = Vec::new();
        let mut refused = Vec::new();
        let mut text_from_stdin = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let value = arg.to_string_lossy();
            if value == "--attach" {
                named = true;
                paths.extend(args.filter(|arg| !arg.is_empty()).map(PathBuf::from));
                break;
            }
            if let Some(path) = value.strip_prefix("--attach=") {
                named = true;
                capture = true;
                if !path.is_empty() {
                    paths.push(PathBuf::from(path));
                }
                continue;
            }
            if value == "--attach-text" {
                named = true;
                capture = false;
                match args.next() {
                    Some(source) if source == "-" => text_from_stdin = true,
                    _ => refused.push(HandOverRefusal {
                        name: "the text".to_owned(),
                        reason: "text is read from standard input only (--attach-text -)"
                            .to_owned(),
                    }),
                }
                continue;
            }
            if value.starts_with("--") {
                capture = false;
                continue;
            }
            if capture && !value.is_empty() {
                paths.push(PathBuf::from(arg));
            }
        }
        if !named {
            return None;
        }
        let mut hand_over = Self {
            paths: paths.into_iter().map(|path| cwd.join(path)).collect(),
            text: None,
            refused,
        };
        if text_from_stdin {
            hand_over.read_text(stdin);
        }
        Some(hand_over.admit())
    }

    fn read_text(&mut self, stdin: impl Read) {
        let mut bytes = Vec::new();
        let read = stdin
            .take(MAX_TEXT_BYTES as u64 + 1)
            .read_to_end(&mut bytes);
        match read {
            Err(error) => self.refuse_text(&format!("it could not be read ({error})")),
            Ok(_) if bytes.len() > MAX_TEXT_BYTES => self.refuse_text(TEXT_TOO_LONG),
            Ok(_) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if !text.trim().is_empty() {
                    self.text = Some(HandOverText { text, from: None });
                }
            }
        }
    }

    fn refuse_text(&mut self, reason: &str) {
        self.refused.push(HandOverRefusal {
            name: "the text".to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// The checks the running app applies to every hand-over, wherever it came from: absolute paths that
    /// exist, no duplicates, at most [`MAX_ITEMS`]; text within [`MAX_TEXT_BYTES`] and not blank.
    #[must_use]
    pub fn admit(mut self) -> Self {
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        let mut over = 0_usize;
        for path in std::mem::take(&mut self.paths) {
            let name = path.display().to_string();
            if !path.is_absolute() {
                self.refused.push(refusal(name, "it is not a full path"));
                continue;
            }
            let path = std::path::absolute(&path).unwrap_or(path);
            if std::fs::metadata(&path).is_err() {
                self.refused.push(refusal(name, "it does not exist"));
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            if kept.len() == MAX_ITEMS {
                over += 1;
                continue;
            }
            kept.push(path);
        }
        if over > 0 {
            self.refused.push(refusal(
                format!("{over} more"),
                &format!("at most {MAX_ITEMS} files and folders are added at a time"),
            ));
        }
        self.paths = kept;
        if let Some(text) = self.text.take() {
            if text.text.len() > MAX_TEXT_BYTES {
                self.refuse_text(TEXT_TOO_LONG);
            } else if !text.text.trim().is_empty() {
                self.text = Some(HandOverText {
                    text: text.text,
                    from: text
                        .from
                        .map(|from| from.chars().take(80).collect::<String>())
                        .filter(|from| !from.trim().is_empty()),
                });
            }
        }
        self.refused.truncate(MAX_ITEMS + 2);
        self
    }

    /// The wire form for the single-instance channel.
    ///
    /// # Errors
    /// Serialization errors (none for these types in practice).
    pub fn encode(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads the wire form; refuses unknown fields and anything larger than [`MAX_WIRE_BYTES`].
    ///
    /// # Errors
    /// The parse error, or a size error.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err("hand-over too large".to_owned());
        }
        serde_json::from_slice(bytes).map_err(|error| error.to_string())
    }

    /// Splits the admitted paths into files and folders (as they are now on disk).
    #[must_use]
    pub fn split_paths(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        self.paths.iter().cloned().partition(|path| !path.is_dir())
    }
}

/// What the window receives for one hand-over: files and folders apart (as they are on disk now), the text, and
/// what was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandOverView {
    /// Files admitted by the same core reader as the native picker.
    pub files: Vec<HandOverFile>,
    /// Absolute paths of folders.
    pub folders: Vec<String>,
    /// The handed-over text, if any.
    pub text: Option<HandOverText>,
    /// What was not admitted, with the reason.
    pub refused: Vec<HandOverRefusal>,
}

/// One OS-selected file and its document-store entry; the path is only used to avoid duplicate chips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandOverFile {
    /// Original absolute path selected outside the app.
    pub path: String,
    /// Document id and facts; the window uses its usual attachment controller to save and read it.
    pub document: vw_core_api::DocumentView,
}

impl HandOverView {
    /// Admits OS-selected files through the native picker's checks, without sending a turn.
    pub async fn read(hand_over: HandOver, core: &vw_core_api::Core) -> Self {
        let (files, folders) = hand_over.split_paths();
        let mut view = Self {
            files: Vec::new(),
            folders: folders
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            text: hand_over.text,
            refused: hand_over.refused,
        };
        for path in files {
            // Composer origin returns facts to this caller; its event does not also place a drop chip.
            let (added, refused) = core
                .add_documents(vec![path.clone()], vw_core_api::DocumentSource::Composer)
                .await;
            view.files
                .extend(added.into_iter().map(|document| HandOverFile {
                    path: path.to_string_lossy().into_owned(),
                    document,
                }));
            view.refused
                .extend(refused.into_iter().map(|r| HandOverRefusal {
                    name: r.name,
                    reason: r.reason,
                }));
        }
        view
    }
}

const TEXT_TOO_LONG: &str = "it is longer than 1 MB";

fn refusal(name: String, reason: &str) -> HandOverRefusal {
    HandOverRefusal {
        name,
        reason: reason.to_owned(),
    }
}

type Notify = Box<dyn Fn() -> bool + Send>;

/// Hand-overs waiting for the window. The single-instance listener and the starting app put them here; the
/// window is told (its subscription) and takes them all at once.
#[derive(Default)]
pub struct HandOverInbox {
    state: Mutex<InboxState>,
}

#[derive(Default)]
struct InboxState {
    pending: Vec<HandOver>,
    notify: Option<Notify>,
}

impl HandOverInbox {
    /// Keeps `hand_over` for the window and tells its subscriber. Empty hand-overs are ignored; past
    /// [`MAX_PENDING`] waiting ones the new one is dropped.
    pub fn push(&self, hand_over: HandOver) {
        if hand_over.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.pending.len() >= MAX_PENDING {
            tracing::warn!("a hand-over was dropped: too many are waiting for the window");
            return;
        }
        state.pending.push(hand_over);
        if state.notify.as_ref().is_some_and(|notify| !notify()) {
            state.notify = None;
        }
    }

    /// Sets the window's subscriber (replacing an earlier one, as a reload does); it is called at once when
    /// hand-overs are already waiting (a launch with `--attach` before the window was ready). The subscriber
    /// returns false when it is gone.
    pub fn subscribe(&self, notify: impl Fn() -> bool + Send + 'static) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let live = state.pending.is_empty() || notify();
        state.notify = live.then(|| Box::new(notify) as Notify);
    }

    /// Takes every waiting hand-over, oldest first.
    pub fn take(&self) -> Vec<HandOver> {
        std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pending,
        )
    }
}

#[cfg(test)]
#[path = "hand_over_tests.rs"]
mod tests;
