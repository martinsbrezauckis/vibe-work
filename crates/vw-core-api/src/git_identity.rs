//! App commit identity in a private atomic JSON file; called by Git Settings, Commit and init.
//! Depends on vw-git identity validation and vw-platform storage. Never writes global Git config.
use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::{GitHub, git_error};
use crate::git_view::{GitIdentitySettings, GitIdentitySource};
use vw_git::identity::{self, CommitIdentity};

const FILE: &str = "identity.json";

/// Reads saved app values, or read-only global Git values before the first Save.
pub(crate) fn load(git: &GitHub) -> Result<GitIdentitySettings, CoreError> {
    let folder = git.folder(".")?;
    let path = folder.join(FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let identity = serde_json::from_slice(&bytes).map_err(|e| {
                CoreError::new(
                    ErrorCode::Internal,
                    format!("Cannot read Git identity settings: {e}"),
                )
            })?;
            Ok(GitIdentitySettings {
                identity,
                source: GitIdentitySource::App,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let (name, email) = identity::read_global(&folder).map_err(git_error)?;
            let source = if name.is_some() || email.is_some() {
                GitIdentitySource::Global
            } else {
                GitIdentitySource::Unset
            };
            Ok(GitIdentitySettings {
                identity: CommitIdentity {
                    name: name.unwrap_or_default(),
                    email: email.unwrap_or_default(),
                },
                source,
            })
        }
        Err(e) => Err(CoreError::new(
            ErrorCode::Internal,
            format!("Cannot read Git identity settings: {e}"),
        )),
    }
}

/// Saves only private app data; both empty fields explicitly disable global prefill.
pub(crate) fn save(
    git: &GitHub,
    identity: &CommitIdentity,
) -> Result<GitIdentitySettings, CoreError> {
    let identity = CommitIdentity {
        name: identity.name.trim().to_owned(),
        email: identity.email.trim().to_owned(),
    };
    if !(identity.name.is_empty() && identity.email.is_empty()) {
        identity::validate(&identity).map_err(git_error)?;
    }
    let folder = git.folder(".")?;
    let bytes = serde_json::to_vec_pretty(&identity)
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("Git identity settings: {e}")))?;
    vw_platform::replace_private(&folder.join(FILE), &bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("Cannot save Git identity settings: {e}"),
        )
    })?;
    Ok(GitIdentitySettings {
        identity,
        source: GitIdentitySource::App,
    })
}
