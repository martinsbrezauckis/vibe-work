//! Folder-check outcomes shared by the hook and core; safe causes and recovery reach the agent, OS details stay in logs.
use std::fmt::Display;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct CopyEscape {
    cwd: PathBuf,
    copy: PathBuf,
}

impl Display for CopyEscape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "session folder {} resolves outside its copy {}",
            self.cwd.display(),
            self.copy.display()
        )
    }
}

impl std::error::Error for CopyEscape {}

pub(super) fn outside_copy(cwd: &Path, copy: PathBuf) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        CopyEscape {
            cwd: cwd.to_owned(),
            copy,
        },
    )
}

macro_rules! folder_message {
    ($folder:literal) => {
        concat!("Vibe Work could not resolve ", $folder, ". Ask the user to check that the chat folders and their links are accessible, then reopen this chat.")
    };
}

/// Name the failing folder when the resolver retained it, without exposing its OS diagnostic.
#[must_use]
pub fn folder_resolution(error: &std::io::Error) -> String {
    if let Some(escape) = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<CopyEscape>())
    {
        return format!(
            "The chat folder {} resolves outside its copy {}. Ask the user to check the folder's links, then reopen this chat in its copy.",
            escape.cwd.display(),
            escape.copy.display()
        );
    }
    super::resolution::failed_path(error).map_or_else(
        || Failure::ChatFolders.text().to_owned(),
        |path| format!(folder_message!("chat folder {}"), path.display()),
    )
}

/// A distinct failure to establish folder access, without exposing transport or storage details.
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    /// Windows did not provide the chat's folder context.
    WindowsContext,
    /// A chat folder or its links could not be resolved.
    ChatFolders,
    /// The core could not be reached for a copy decision.
    CopyUnavailable,
    /// The copy reply did not include a complete decision.
    CopyAnswer,
    /// No chat could be identified as the copy's owner.
    CopyOwner,
    /// Saved copy records could not be opened.
    RecordsOpen,
    /// Saved copy records could not be read.
    RecordsRead,
    /// The copy question did not identify a valid chat and folder.
    CopyRequest,
    /// The chat's agent tools are unavailable.
    ToolsUnavailable,
    /// The copy decision task could not finish.
    CopyTask,
}

impl Failure {
    /// The agent's diagnosis and next step for this outcome.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::WindowsContext => {
                "Vibe Work could not identify this chat's folder. Ask the user to reopen this chat before using its files."
            }
            Self::ChatFolders => {
                folder_message!("a chat folder")
            }
            Self::CopyUnavailable => {
                "Vibe Work could not check who uses this copy. Ask the user to reopen this chat before using the copy."
            }
            Self::CopyAnswer => {
                "Vibe Work did not provide a complete access decision for this copy. Ask the user to reopen this chat before using the copy."
            }
            Self::CopyOwner => {
                "Vibe Work cannot tell which chat uses this copy. Ask the user to open a chat in the copy they want you to use."
            }
            Self::RecordsOpen => {
                "Vibe Work could not open the saved list of chat copies. Ask the user to reopen this chat before using the copy."
            }
            Self::RecordsRead => {
                "Vibe Work could not read the saved list of chat copies. Ask the user to reopen this chat before using the copy."
            }
            Self::CopyRequest => {
                "Vibe Work could not identify the chat and copy to check. Ask the user to reopen this chat before using the copy."
            }
            Self::ToolsUnavailable => {
                "This chat's tools are unavailable. Ask the user to reopen this chat before using the copy."
            }
            Self::CopyTask => {
                "Vibe Work could not finish checking access to this copy. Try once more; if it still fails, ask the user to reopen this chat."
            }
        }
    }

    /// Keep the original diagnostic in the process log and return the agent-safe outcome.
    #[must_use]
    pub fn report(self, detail: impl Display) -> String {
        eprintln!("folder check {self:?}: {detail}");
        self.text().to_owned()
    }
}
