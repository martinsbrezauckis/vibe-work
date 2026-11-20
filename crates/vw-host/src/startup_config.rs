//! Vibe Work's startup fixes to its own `VIBE_HOME/config.toml`, applied as one all-or-nothing write.
//!
//! The steps edit the text in memory, in launch order: the experiment and catalog defaults
//! ([`crate::experiment_pins`]), the Large 4 offline fallback and catalog cleanup
//! ([`crate::default_model`]; catalog-off product launch or first catalog-on launch in either mode),
//! extension paths ([`crate::extension_paths`]), the session-title model ([`crate::title_model`]), the commit
//! credit line ([`crate::commit_signature`]) and the system trust store ([`crate::trust_store`]).
//! Each step sees the text the steps before it produced, so the result equals running
//! them one after another on the file.
//!
//! The file is read once and written at most once ([`Draft`]):
//! - a link, a non-file or a file over 1 MiB is never written; no step edits it;
//! - just before the write the file is read again: a file that changed meanwhile (or appeared, when it was missing)
//!   is not written;
//! - an existing file is replaced through a new user-only file and a rename, a missing one is created user-only, so a
//!   failed write leaves the file as it was.
//!
//! A step that cannot edit the text keeps its own reason in its outcome and leaves the text to the next step. A
//! refused or failed write changes nothing for any step. Failure rule: only the Large 4 step fails the product launch
//! (the file cannot be read, or its own change could not be written); the other steps log.
//!
//! Depends on: the step modules (pure text edits), `vw_platform` private files, tracing.
//! Called by: [`crate::launch`] ([`prepare_product`], [`prepare_dev`]) and each step's own `ensure`.

use std::io;
use std::path::{Path, PathBuf};

use crate::commit_signature::CommitSignature;
use crate::default_model::ModelDefaults;
use crate::title_model::TitleModel;
use crate::trust_store::TrustStore;

/// The largest config the startup steps edit.
const MAX_LEN: u64 = 1024 * 1024;

/// The reason a config that changed between the read and the write is left alone.
const CHANGED: &str = "config.toml changed while Vibe Work prepared it";

/// The config file as read: missing, its text, or a file the steps never write (with the reason).
#[derive(Debug, PartialEq, Eq)]
enum Found {
    Missing,
    Text(String),
    Refused(String),
}

fn read_guarded(path: &Path) -> io::Result<Found> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Found::Missing),
        Err(e) => return Err(e),
    };
    if metadata.file_type().is_symlink() {
        return Ok(Found::Refused("config.toml is a link".to_owned()));
    }
    if !metadata.is_file() {
        return Ok(Found::Refused("config.toml is not a file".to_owned()));
    }
    if metadata.len() > MAX_LEN {
        return Ok(Found::Refused("config.toml is over 1 MiB".to_owned()));
    }
    std::fs::read_to_string(path).map(Found::Text)
}

/// One step's outcome, and whether its edit changed the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Staged<T> {
    pub(crate) outcome: T,
    pub(crate) changed: bool,
}

/// What became of the combined edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Written {
    /// No step changed the text: nothing was written.
    Unchanged,
    /// The combined edit was written.
    Saved,
    /// Not written, with the reason (a link, a non-file, over 1 MiB, or changed meanwhile).
    Refused(String),
    /// The write failed, with the error; the file is as it was.
    Failed(String),
}

impl Written {
    /// Whether the file now holds every step's edit (written, or nothing to write).
    pub(crate) fn landed(&self) -> bool {
        matches!(self, Self::Unchanged | Self::Saved)
    }

    /// A step's outcome after the write: its own outcome when its edit landed (or it made none), `refused(why)` when
    /// the write was refused, the error when the write failed.
    ///
    /// # Errors
    /// The write error, when this step's edit was part of a failed write.
    pub(crate) fn settle<T>(
        &self,
        staged: Staged<T>,
        refused: impl FnOnce(String) -> T,
    ) -> Result<T, String> {
        if !staged.changed {
            return Ok(staged.outcome);
        }
        match self {
            Self::Unchanged | Self::Saved => Ok(staged.outcome),
            Self::Refused(why) => Ok(refused(why.clone())),
            Self::Failed(error) => Err(error.clone()),
        }
    }
}

/// The config read once, edited by the steps in memory, and written at most once ([`Draft::commit`]).
#[derive(Debug)]
pub(crate) struct Draft {
    vibe_home: PathBuf,
    path: PathBuf,
    /// The text as read (`None`: no file).
    original: Option<String>,
    text: String,
    /// Why no step may edit this file.
    refused: Option<String>,
}

impl Draft {
    /// Reads `vibe_home/config.toml` (a missing file is an empty draft).
    ///
    /// # Errors
    /// The I/O error when the file exists but cannot be inspected or read as UTF-8 text.
    pub(crate) fn open(vibe_home: &Path) -> io::Result<Self> {
        let path = vibe_home.join("config.toml");
        let (original, refused) = match read_guarded(&path)? {
            Found::Missing => (None, None),
            Found::Text(text) => (Some(text), None),
            Found::Refused(why) => (None, Some(why)),
        };
        Ok(Self {
            vibe_home: vibe_home.to_path_buf(),
            text: original.clone().unwrap_or_default(),
            path,
            original,
            refused,
        })
    }

    /// Runs one step's `edit` over the current text. `edit` gets the text and whether the file is missing and no step
    /// wrote any text yet; it returns the new text (or `None`) and its outcome. On a file no step may edit,
    /// `refused(why)` is the outcome and the text stays.
    pub(crate) fn apply<T>(
        &mut self,
        edit: impl FnOnce(&str, bool) -> (Option<String>, T),
        refused: impl FnOnce(String) -> T,
    ) -> Staged<T> {
        if let Some(why) = &self.refused {
            return Staged {
                outcome: refused(why.clone()),
                changed: false,
            };
        }
        let missing = self.original.is_none() && self.text.is_empty();
        let (text, outcome) = edit(&self.text, missing);
        let changed = text.as_ref().is_some_and(|text| *text != self.text);
        if let Some(text) = text {
            self.text = text;
        }
        Staged { outcome, changed }
    }

    /// Writes the combined edit once (see the module docs).
    pub(crate) fn commit(self) -> Written {
        self.commit_with(write_config)
    }

    /// [`Draft::commit`] with the file write given: `write(path, bytes, replace)` replaces an existing file when
    /// `replace`, or creates a new one (an `AlreadyExists` error means the file appeared meanwhile).
    pub(crate) fn commit_with(
        self,
        write: impl FnOnce(&Path, &[u8], bool) -> io::Result<()>,
    ) -> Written {
        if let Some(why) = self.refused {
            return Written::Refused(why);
        }
        let unchanged = match &self.original {
            Some(original) => *original == self.text,
            None => self.text.is_empty(),
        };
        if unchanged {
            return Written::Unchanged;
        }
        let expected = self.original.clone().map_or(Found::Missing, Found::Text);
        match read_guarded(&self.path) {
            Ok(now) if now == expected => {}
            Ok(_) => return Written::Refused(CHANGED.to_owned()),
            Err(error) => return Written::Failed(format!("{}: {error}", self.path.display())),
        }
        let replace = self.original.is_some();
        if !replace && let Err(error) = std::fs::create_dir_all(&self.vibe_home) {
            return Written::Failed(format!("{}: {error}", self.vibe_home.display()));
        }
        match write(&self.path, self.text.as_bytes(), replace) {
            Ok(()) => Written::Saved,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                Written::Refused(CHANGED.to_owned())
            }
            Err(error) => Written::Failed(error.to_string()),
        }
    }
}

/// Replaces `path` atomically, or creates it user-only when `replace` is false (never over an existing file).
fn write_config(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    if replace {
        return vw_platform::replace_private(path, bytes).map_err(io::Error::other);
    }
    match vw_platform::write_private_new(path, bytes) {
        Ok(()) => Ok(()),
        Err(vw_platform::PrivateFileError::Io { source, .. })
            if source.kind() == io::ErrorKind::AlreadyExists =>
        {
            Err(source)
        }
        Err(error) => Err(io::Error::other(error)),
    }
}

/// The product launch's startup fixes, catalog policy before model cleanup; each outcome is logged.
///
/// # Errors
/// The I/O error when the config cannot be read, or when the Large 4 entry's own change could not be written. A
/// refused write (link, size, changed meanwhile) is logged and does not fail the launch.
pub(crate) fn prepare_product(vibe_home: &Path) -> io::Result<()> {
    prepare(vibe_home, true, write_config)
}

/// The development launch's startup fixes (its template config already holds the model entries); every outcome,
/// including a read error, is logged and never stops the launch.
pub(crate) fn prepare_dev(vibe_home: &Path) {
    if let Err(error) = prepare(vibe_home, false, write_config) {
        tracing::warn!(%error, "startup config: config.toml could not be updated");
    }
}

/// [`prepare_product`] (`product`) or [`prepare_dev`] with the file write given (see [`Draft::commit_with`]).
pub(crate) fn prepare(
    vibe_home: &Path,
    product: bool,
    write: impl FnOnce(&Path, &[u8], bool) -> io::Result<()>,
) -> io::Result<()> {
    let mut draft = match Draft::open(vibe_home) {
        Ok(draft) => draft,
        Err(error) if product => return Err(error),
        Err(error) => {
            tracing::warn!(%error, "startup config: config.toml could not be read; left unchanged");
            return Ok(());
        }
    };
    let pins = crate::experiment_pins::stage(&mut draft, vibe_home);
    let catalog_on = draft
        .text
        .parse::<toml::Table>()
        .ok()
        .and_then(|config| config.get("experimental_enable_model_catalog")?.as_bool())
        == Some(true);
    let model = (if catalog_on {
        pins.catalog_first_launch()
    } else {
        product
    })
    .then(|| draft.apply(crate::default_model::edit, |_| ModelDefaults::Unchanged));
    let paths = draft.apply(|text, _| crate::extension_paths::edit(text), Err);
    // An unreadable marker skips only the one-time replacement; the step still adds a missing title model.
    let title_pending = crate::title_model::migration_pending(vibe_home).unwrap_or_else(|error| {
        tracing::warn!(%error, "session titles: migration marker could not be checked");
        false
    });
    let title = draft.apply(
        |text, _| crate::title_model::edit(text, title_pending),
        TitleModel::Skipped,
    );
    let signature = draft.apply(
        |text, _| crate::commit_signature::edit(text),
        CommitSignature::Skipped,
    );
    let trust = draft.apply(
        |text, _| crate::trust_store::edit(text),
        TrustStore::Skipped,
    );
    let written = draft.commit_with(write);
    crate::extension_paths::log(written.settle(paths, Err).and_then(|outcome| outcome));
    let title = written.settle(title, TitleModel::Skipped);
    if let Ok(outcome) = &title
        && let Err(error) =
            crate::title_model::finish(vibe_home, title_pending, outcome, written.landed())
    {
        tracing::warn!(%error, "session titles: migration marker could not be written");
    }
    crate::startup_config_log::title_model(title);
    crate::startup_config_log::commit_signature(
        written.settle(signature, CommitSignature::Skipped),
    );
    crate::startup_config_log::trust_store(written.settle(trust, TrustStore::Skipped));
    crate::startup_config_log::experiment_pins(crate::experiment_pins::finish(pins, &written));
    if let Some(model) = model {
        let settled = written.settle(model, |why| {
            tracing::warn!(%why, "model defaults: config not changed");
            ModelDefaults::Unchanged
        });
        if let Err(error) = settled {
            return Err(io::Error::other(error));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_config_tests.rs"]
mod tests;
