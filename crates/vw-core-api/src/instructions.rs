//! Instructions in the core: reads and writes the `AGENTS.md` files Vibe puts in the
//! system prompt. The personal one lives in Vibe Work's own `VIBE_HOME` (the app has its
//! own home, so the command-line tool's `~/.vibe/AGENTS.md` does not apply here); the project one
//! lives in the folder of a session.
//!
//! Depends on: `instructions_view` (request and answer types), `core::Shared` (session folders, the
//! home paths), `error`. Called by: `dispatch` (`Command::Instructions`).
//! Invariants:
//! - The project file is `<session folder>/AGENTS.md`, the folder from the core's own record of the
//!   session, never a path from the client. The personal file is `<own VIBE_HOME>/AGENTS.md`.
//! - The user's command-line home is only ever READ (one file, on `readCli`, plus a size-free
//!   existence check for the "can copy" hint). No code path here writes, renames or deletes in it.
//! - A write replaces the file with exactly the given text, through a temporary file in the same
//!   folder and a rename. It is refused when the file is a link (a repository can plant a link, and
//!   a write through it would reach any file the user can write), when the file no longer holds what
//!   the user saw (`expected`), or when the text is too large or holds a NUL.
//! - Every origin reads and writes directly (the Debug API acts as the user; the dialog is
//!   gone). Text and paths are never logged (only lengths).
//! - The personal file may start with Vibe Work's own note (`working_note`): a read never shows it and a
//!   write keeps it in front of the user's text, so the user only sees and edits their own instructions.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::instructions_view::{
    InstructionsDoc, InstructionsRequest, InstructionsResult, InstructionsScope,
};

/// File name Vibe reads in every folder and in its home.
const FILE_NAME: &str = "AGENTS.md";
/// Largest file the app will show or replace.
const MAX_READ_BYTES: u64 = 1024 * 1024;
/// Largest text one write accepts.
const MAX_WRITE_BYTES: usize = 256 * 1024;

/// Where the instruction files live, fixed when the core starts.
#[derive(Debug, Clone, Default)]
pub(crate) struct InstructionsPaths {
    /// Vibe Work's own `VIBE_HOME` (the supervisor's).
    pub(crate) own_home: PathBuf,
    /// The user's Vibe command-line home, when this run may look at it.
    pub(crate) cli_home: Option<PathBuf>,
}

/// Tuning of the instructions part of a core.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct InstructionsConfig {
    /// The user's Vibe command-line home (`~/.vibe`, or the folder `VIBE_HOME` names) for the
    /// "copy my CLI instructions" offer. `None` (the default; tests and runs that must not touch the
    /// user's home): the offer is not available. The app sets it; development runs point it at
    /// their own scratch home.
    pub cli_home: Option<PathBuf>,
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

/// A refusal with a stable reason word in `server_code`.
fn refuse(reason: &str, message: impl Into<String>) -> CoreError {
    let mut error = CoreError::new(ErrorCode::Denied, message);
    error.server_code = Some(reason.to_owned());
    error
}

/// Reads the file at `path` as a document (never follows a link).
fn read_doc(scope: InstructionsScope, path: &Path, folder: Option<String>) -> InstructionsDoc {
    let mut doc = InstructionsDoc {
        scope,
        path: path.to_string_lossy().into_owned(),
        folder,
        exists: false,
        text: String::new(),
        editable: true,
        note: None,
        cli_available: None,
    };
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return doc,
        Err(error) => {
            doc.editable = false;
            doc.note = Some(format!("The file cannot be read: {}.", error.kind()));
            return doc;
        }
    };
    doc.exists = true;
    if meta.file_type().is_symlink() {
        doc.editable = false;
        doc.note = Some(
            "This file is a link to another file. Vibe Work does not write through links: edit the file it points to.".to_owned(),
        );
    } else if !meta.is_file() {
        doc.editable = false;
        doc.note = Some("AGENTS.md here is a folder, not a file.".to_owned());
    } else if meta.len() > MAX_READ_BYTES {
        doc.editable = false;
        doc.note = Some("The file is over 1 MB. Edit it in your own editor.".to_owned());
    } else {
        match fs::read(path).map(String::from_utf8) {
            Ok(Ok(text)) if scope == InstructionsScope::Personal => {
                // Vibe Work's own note is not the user's text: a file holding only the note reads as no file.
                doc.text = crate::working_note::strip(&text);
                doc.exists = !doc.text.is_empty() || !crate::working_note::is_ours(&text);
            }
            Ok(Ok(text)) => doc.text = text,
            Ok(Err(_)) => {
                doc.editable = false;
                doc.note = Some("The file is not plain UTF-8 text, so it is not shown.".to_owned());
            }
            Err(error) => {
                doc.editable = false;
                doc.note = Some(format!("The file cannot be read: {}.", error.kind()));
            }
        }
    }
    doc
}

/// Adds the "a CLI file can be copied" hint to a personal document (project documents get none).
fn with_cli(mut doc: InstructionsDoc, cli_home: Option<&Path>) -> InstructionsDoc {
    if doc.scope == InstructionsScope::Personal {
        doc.cli_available =
            Some(cli_home.is_some_and(|home| cli_text(&home.join(FILE_NAME)).is_some()));
    }
    doc
}

/// True when `path` is a regular, non-empty, readable text file (the CLI copy offer).
fn cli_text(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    // A link is followed here on purpose: the user's own CLI file may be one (dotfile managers),
    // and this is a read of a file the user already owns. Size is bounded.
    let target = if meta.file_type().is_symlink() {
        fs::metadata(path).ok()?
    } else {
        meta
    };
    if !target.is_file() || target.len() > MAX_READ_BYTES {
        return None;
    }
    let text = String::from_utf8(fs::read(path).ok()?).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// Writes `text` into `path` through a temporary file next to it.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent folder"))?;
    fs::create_dir_all(dir)?;
    let existing = fs::metadata(path).ok();
    let tmp = dir.join(format!(".AGENTS.md.{}.tmp", std::process::id()));
    // `create_new` refuses an existing name, a link included: the temporary file is always ours.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Some(meta) = &existing {
            // Keep the file's permissions (a read-only or shared file stays that way).
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The comparable form of "what the user saw": a missing file and an empty one are the same.
fn normalized(text: Option<&str>) -> &str {
    text.unwrap_or("")
}

/// The pure part of a write, run off the async threads: checks the file, then replaces it.
fn write_checked(
    scope: InstructionsScope,
    path: &Path,
    folder: Option<String>,
    text: &str,
    expected: Option<&str>,
    cli: Option<&Path>,
) -> Result<InstructionsDoc, CoreError> {
    let current = read_doc(scope, path, folder.clone());
    if current.exists && !current.editable {
        return Err(refuse(
            "notEditable",
            current
                .note
                .unwrap_or_else(|| "this file cannot be edited here".to_owned()),
        ));
    }
    // The personal file is compared as it is on disk: it must be exactly what the user was shown, or our exact
    // note followed by it (`working_note::shows_as`), so no text the user did not see can slip through a save.
    let raw = if scope == InstructionsScope::Personal && current.exists {
        Some(fs::read_to_string(path).unwrap_or_default())
    } else {
        None
    };
    let stale = match &raw {
        Some(raw) => !crate::working_note::shows_as(raw, normalized(expected)),
        None => normalized(Some(&current.text)) != normalized(expected),
    };
    if stale {
        return Err(refuse(
            "changedOnDisk",
            "AGENTS.md changed after it was loaded. Reload it and review the difference again.",
        ));
    }
    if !current.exists && text.is_empty() {
        return Ok(with_cli(current, cli));
    }
    // The personal file keeps Vibe Work's note in front of the user's text when it had one.
    let composed = if scope == InstructionsScope::Personal {
        let existing = raw.unwrap_or_else(|| fs::read_to_string(path).unwrap_or_default());
        crate::working_note::keep(&existing, text)
    } else {
        text.to_owned()
    };
    write_atomic(path, &composed).map_err(|error| {
        CoreError::new(
            ErrorCode::Internal,
            format!("AGENTS.md could not be written: {}", error.kind()),
        )
    })?;
    Ok(with_cli(read_doc(scope, path, folder), cli))
}

impl Core {
    /// Executes one instructions request for `origin`.
    pub(crate) async fn instructions(
        &self,
        origin: Origin,
        request: InstructionsRequest,
    ) -> Result<CommandResult, CoreError> {
        let result = match request {
            InstructionsRequest::Read { scope, session_id } => {
                let (path, folder) = self.target(scope, session_id.as_deref())?;
                let cli = self.shared.instructions.cli_home.clone();
                let doc =
                    blocking(move || with_cli(read_doc(scope, &path, folder), cli.as_deref()))
                        .await?;
                InstructionsResult::Doc { doc }
            }
            InstructionsRequest::ReadCli {} => self.read_cli().await?,
            InstructionsRequest::Write {
                scope,
                session_id,
                text,
                expected,
            } => {
                let doc = self
                    .write(origin, scope, session_id.as_deref(), text, expected)
                    .await?;
                InstructionsResult::Doc { doc }
            }
            InstructionsRequest::Apply { scope, session_id } => {
                self.apply(origin, scope, session_id.as_deref()).await?
            }
        };
        Ok(CommandResult::Instructions { result })
    }

    /// `readCli`: reads the CLI home's personal file with the user's rights.
    async fn read_cli(&self) -> Result<InstructionsResult, CoreError> {
        let home = self.shared.instructions.cli_home.clone().ok_or_else(|| {
            refuse(
                "notConfigured",
                "this run does not look at the command-line home",
            )
        })?;
        let text = blocking(move || cli_text(&home.join(FILE_NAME))).await?;
        Ok(InstructionsResult::Cli {
            available: text.is_some(),
            text: text.unwrap_or_default(),
        })
    }

    /// `write`: validates the text and replaces the file with the user's rights.
    async fn write(
        &self,
        origin: Origin,
        scope: InstructionsScope,
        session_id: Option<&str>,
        text: String,
        expected: Option<String>,
    ) -> Result<InstructionsDoc, CoreError> {
        if text.len() > MAX_WRITE_BYTES {
            return Err(invalid(format!(
                "the text is {} bytes; at most {MAX_WRITE_BYTES} are accepted",
                text.len()
            )));
        }
        if text.contains('\0') {
            return Err(invalid("the text holds a NUL character"));
        }
        let (path, folder) = self.target(scope, session_id)?;
        let cli = self.shared.instructions.cli_home.clone();
        let doc = blocking(move || {
            write_checked(
                scope,
                &path,
                folder,
                &text,
                expected.as_deref(),
                cli.as_deref(),
            )
        })
        .await??;
        tracing::info!(
            origin = origin.as_str(),
            scope = ?scope,
            characters = doc.text.chars().count(),
            "core: instructions written"
        );
        Ok(doc)
    }

    /// Closes the channels of the sessions that should take the current instruction files.
    async fn apply(
        &self,
        origin: Origin,
        scope: InstructionsScope,
        session_id: Option<&str>,
    ) -> Result<InstructionsResult, CoreError> {
        let folder = match scope {
            InstructionsScope::Personal => None,
            InstructionsScope::Project => self.target(scope, session_id)?.1,
        };
        self.apply_instruction_folder(origin, scope, folder).await
    }

    /// Refreshes sessions whose prompt may have included a changed project file.
    pub(crate) async fn apply_project_instruction_folder(
        &self,
        origin: Origin,
        folder: &str,
    ) -> Result<InstructionsResult, CoreError> {
        self.apply_instruction_folder(origin, InstructionsScope::Project, Some(folder.to_owned()))
            .await
    }

    async fn apply_instruction_folder(
        &self,
        origin: Origin,
        scope: InstructionsScope,
        folder: Option<String>,
    ) -> Result<InstructionsResult, CoreError> {
        let (mut reopened, mut waiting) = (0_u32, 0_u32);
        for open in self.shared.channels.open_channels() {
            let id = open.session_id;
            if let Some(folder) = &folder
                && self.shared.names.cwd_of(&self.shared.store, &id).as_deref() != Some(folder)
            {
                continue;
            }
            if never_used(&self.shared, &id) {
                // A session that never ran a turn has built nothing yet: a config reload makes its
                // first turn use the current files (observed live). Closing it would
                // not help: the idle rules never close a session without a turn, and a reopened one
                // would start from the state it was created with.
                let params = json!({"sessionId": id, "reloadRuntime": true});
                if self
                    .forward(origin, "config/reload", params, None)
                    .await
                    .is_ok()
                {
                    reopened += 1;
                }
            } else if crate::recovery::quiet(&self.shared, &id) {
                if self.shared.channels.close(&id).await {
                    reopened += 1;
                }
            } else {
                self.shared.channels.close_when_quiet(&id);
                waiting += 1;
            }
        }
        tracing::info!(scope = ?scope, reopened, waiting, "core: instructions applied");
        Ok(InstructionsResult::Applied { reopened, waiting })
    }

    /// The file a request is about, and the folder for a project one.
    fn target(
        &self,
        scope: InstructionsScope,
        session_id: Option<&str>,
    ) -> Result<(PathBuf, Option<String>), CoreError> {
        match scope {
            InstructionsScope::Personal => {
                let home = &self.shared.instructions.own_home;
                if home.as_os_str().is_empty() {
                    return Err(refuse("notConfigured", "this run has no Vibe Work home"));
                }
                Ok((home.join(FILE_NAME), None))
            }
            InstructionsScope::Project => {
                let id =
                    session_id.ok_or_else(|| invalid("a project document needs a sessionId"))?;
                let folder = self
                    .shared
                    .names
                    .cwd_of(&self.shared.store, id)
                    .ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::NotFound,
                            format!(
                                "session {id} is not known to the core (list or open it first)"
                            ),
                        )
                    })?;
                let dir = PathBuf::from(&folder);
                if !dir.is_absolute() || !dir.is_dir() {
                    return Err(CoreError::new(
                        ErrorCode::NotFound,
                        "the session's folder is not available",
                    ));
                }
                Ok((dir.join(FILE_NAME), Some(folder)))
            }
        }
    }
}

/// True when the loaded session is idle and has never run a turn (nothing built yet).
fn never_used(shared: &crate::core::Shared, id: &str) -> bool {
    shared.store.view(id).is_some_and(|view| {
        let state = &view.state;
        let status = state
            .pointer("/session/status/type")
            .or_else(|| state.pointer("/session/status"))
            .and_then(serde_json::Value::as_str);
        let empty = |pointer: &str| {
            state
                .pointer(pointer)
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
        };
        status == Some("idle") && empty("/turns") && empty("/activeCallbacks")
    })
}

/// Runs blocking file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CoreError::new(ErrorCode::Internal, "file work was cancelled"))
}
