//! One atomic record per chat for pending, served and fallback choices.
//! Used only by `model_choices`. Private durable replacement commits the whole store; an OS lock
//! and a fresh disk read preserve other windows' chats. Legacy files are immutable rollback snapshots.
use super::Choice;
use crate::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

pub(super) const FILE: &str = "model-choices.json";
const LEGACY: [(&str, &str); 3] = [
    ("pending-models.json", "pending"),
    ("chat-models.json", "remembered"),
    ("model-fallbacks.json", "fallback"),
];

#[derive(Clone, Default, PartialEq, Serialize)]
pub(super) struct Record {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remembered: Option<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Choice>,
}

impl Record {
    pub fn replay(&self) -> Option<Choice> {
        self.fallback.clone().or_else(|| self.pending.clone())
    }

    pub fn expected(&self) -> Option<Choice> {
        self.replay().or_else(|| self.remembered.clone())
    }

    pub fn accepted(&mut self, choice: Choice) {
        self.pending = Some(choice.clone());
        self.remembered = Some(choice);
        self.fallback = None;
    }

    pub fn replaced(&mut self, choice: Choice) {
        self.fallback = Some(choice.clone());
        self.remembered = Some(choice);
        self.pending = None;
    }

    fn read(value: &Value) -> Self {
        let choice = |key: &str| {
            serde_json::from_value::<Choice>(value[key].clone())
                .ok()
                .filter(valid)
        };
        Self {
            pending: choice("pending"),
            remembered: choice("remembered"),
            fallback: choice("fallback"),
        }
    }
}

// Provider-defined strings and labels have no arbitrary length or aggregate size cap. The same
// validation is used on writes and reads, including explicit Off and future provider levels.
fn valid(choice: &Choice) -> bool {
    !choice.alias.is_empty()
        && !choice.effort.is_empty()
        && !choice
            .effort
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    // Keep damaged individual entries verbatim when another chat changes. A bad choice must not
    // discard other chats, nor should reading an unavailable model depend on today's catalog.
    chats: BTreeMap<String, Value>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            chats: BTreeMap::new(),
        }
    }
}

impl Document {
    fn get(&self, id: &str) -> Record {
        self.chats
            .get(id)
            .map_or_else(Record::default, Record::read)
    }
}

pub(super) struct Records {
    state: Mutex<State>,
    dir: Option<PathBuf>,
}

struct State {
    document: Document,
    // A handoff still runs when its copy cannot be written. Carry that copy into the next
    // successful transaction, unless another window has already saved a choice for the new id.
    unsaved_copies: BTreeMap<String, Value>,
}

impl Records {
    pub fn load(dir: Option<&Path>) -> Self {
        Self::load_with(dir, save)
    }

    fn load_with(
        dir: Option<&Path>,
        write: impl FnOnce(&Path, &Document) -> io::Result<()>,
    ) -> Self {
        let state = dir.map(|dir| {
            let locked = lock(dir).inspect_err(|error| {
                tracing::warn!(%error, "saved chat model choices are read-only; commit lock unavailable");
            }).ok();
            // Atomic replacement permits a coherent read even when this instance cannot write.
            let (state, migrated) = read_for_start(dir)?;
            if migrated && locked.is_some() && let Err(error) = write(&dir.join(FILE), &state) {
                // Keep the imported choices usable and retry from untouched legacy files on the
                // next mutation/start. No migration marker is written separately from the record.
                tracing::warn!(%error, "saved chat model choices migration could not be committed");
            }
            Ok::<_, io::Error>(state)
        }).transpose().unwrap_or_else(|error| {
            // A later mutation re-reads under the lock and refuses to overwrite an unreadable
            // store. Starting the app must still let a person open their chats.
            tracing::warn!(%error, "saved chat model choices could not be read; files retained");
            None
        }).unwrap_or_default();
        Self {
            state: Mutex::new(State {
                document: state,
                unsaved_copies: BTreeMap::new(),
            }),
            dir: dir.map(Path::to_path_buf),
        }
    }

    pub fn get(&self, id: &str) -> Record {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .document
            .get(id)
    }

    pub fn named(&self, alias: &str) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for field in ["remembered", "fallback", "pending"] {
            for value in state.document.chats.values() {
                if let Ok(choice) = serde_json::from_value::<Choice>(value[field].clone())
                    && choice.alias == alias
                    && !choice.display_name.is_empty()
                {
                    return Some(choice.display_name);
                }
            }
        }
        None
    }

    pub fn update(&self, id: &str, change: impl FnOnce(&mut Record)) -> Result<(), CoreError> {
        self.transaction(
            |state| {
                let before = state.get(id);
                let mut record = before.clone();
                change(&mut record);
                if record == before {
                    return Ok(false);
                }
                if id.is_empty()
                    || [&record.pending, &record.remembered, &record.fallback]
                        .into_iter()
                        .flatten()
                        .any(|choice| !valid(choice))
                {
                    return Err(io::Error::other("invalid model choice"));
                }
                // Updating this chat intentionally replaces its choice. Unknown/damaged entries for
                // every other chat stay byte-for-byte equivalent JSON values in the document.
                state.chats.insert(id.into(), serde_json::to_value(record)?);
                Ok(true)
            },
            save,
        )
    }

    pub fn forget(&self, id: &str) {
        if let Err(error) = self.transaction(|state| Ok(state.chats.remove(id).is_some()), save) {
            tracing::warn!(%error, "chat model record not removed");
        }
    }

    pub fn copy(&self, old: &str, new: &str) -> Result<(), CoreError> {
        self.transaction(
            |state| {
                let Some(record) = state.chats.get(old).cloned() else {
                    return Ok(false);
                };
                if state.chats.get(new) == Some(&record) {
                    return Ok(false);
                }
                state.chats.insert(new.into(), record);
                Ok(true)
            },
            save,
        )
    }

    pub fn copy_in_memory(&self, old: &str, new: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(record) = state.document.chats.get(old).cloned() {
            state.document.chats.insert(new.into(), record.clone());
            state.unsaved_copies.insert(new.into(), record);
        }
    }

    fn transaction(
        &self,
        change: impl FnOnce(&mut Document) -> io::Result<bool>,
        write: impl FnOnce(&Path, &Document) -> io::Result<()>,
    ) -> Result<(), CoreError> {
        let mut current = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let commit = || -> io::Result<Document> {
            let _lock = self.dir.as_deref().map(lock).transpose()?;
            let (mut next, migrated) = match self.dir.as_deref() {
                Some(dir) => read(dir)?,
                None => (current.document.clone(), false),
            };
            let mut copied = false;
            for (id, record) in &current.unsaved_copies {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    next.chats.entry(id.clone())
                {
                    entry.insert(record.clone());
                    copied = true;
                }
            }
            let changed = change(&mut next)?;
            if (changed || migrated || copied)
                && let Some(dir) = &self.dir
            {
                write(&dir.join(FILE), &next)?;
            }
            Ok(next)
        };
        current.document = commit().map_err(|error| {
            tracing::warn!(%error, "chat model choice commit failed");
            CoreError::new(
                ErrorCode::Internal,
                "The chat's model choice could not be saved. Try picking it again.",
            )
        })?;
        current.unsaved_copies.clear();
        Ok(())
    }
}

fn lock(dir: &Path) -> io::Result<File> {
    vw_platform::ensure_private_dir(dir).map_err(io::Error::other)?;
    let path = dir.join("model-choices.lock");
    let file = match vw_platform::create_private_new(&path) {
        Ok(file) => file,
        Err(vw_platform::PrivateFileError::Io { source, .. })
            if source.kind() == io::ErrorKind::AlreadyExists =>
        {
            OpenOptions::new().read(true).write(true).open(path)?
        }
        Err(error) => return Err(io::Error::other(error)),
    };
    file.lock()?;
    Ok(file)
}

fn read(dir: &Path) -> io::Result<(Document, bool)> {
    match File::open(dir.join(FILE)) {
        Ok(file) => {
            let document: Document = serde_json::from_reader(BufReader::new(file))?;
            if document.version != 1 {
                return Err(io::Error::other("unsupported model choice record version"));
            }
            Ok((document, false))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((migrate(dir)?, true)),
        Err(error) => Err(error),
    }
}

fn read_for_start(dir: &Path) -> io::Result<(Document, bool)> {
    match read(dir) {
        Ok(loaded) => Ok(loaded),
        Err(error) if !dir.join(FILE).try_exists()? => {
            // A broken legacy file blocks committing a migration, but the other two can still
            // restore choices in memory, just as the released reader did. Writes stay strict.
            tracing::warn!(%error, "saved chat model choices legacy file could not be read; files retained");
            Ok((legacy(dir).0, false))
        }
        Err(error) => Err(error),
    }
}

fn migrate(dir: &Path) -> io::Result<Document> {
    let (state, error) = legacy(dir);
    error.map_or(Ok(state), Err)
}

fn legacy(dir: &Path) -> (Document, Option<io::Error>) {
    let mut state = Document::default();
    let mut problem = None;
    for (file, field) in LEGACY {
        let values = match File::open(dir.join(file)) {
            Ok(file) => serde_json::from_reader::<_, BTreeMap<String, Value>>(BufReader::new(file))
                .map_err(io::Error::from),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => Err(error),
        };
        match values {
            Ok(values) => {
                for (id, value) in values {
                    state
                        .chats
                        .entry(id)
                        .or_insert_with(|| serde_json::json!({}))[field] = value;
                }
            }
            Err(error) => {
                problem.get_or_insert(error);
            }
        }
    }
    (state, problem)
}

fn save(file: &Path, state: &Document) -> io::Result<()> {
    vw_platform::replace_private_durable(file, &serde_json::to_vec(state)?)
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests;
