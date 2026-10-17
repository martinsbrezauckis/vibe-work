//! Approval preflight and fail-stop application; uses tempfile for same-folder atomic writes.
//! Called only by `Proposal::apply`. Checks every base before any mutation, without rollback.

use std::fs;
use std::io::Write;

use tempfile::NamedTempFile;

use crate::{ApplyError, Decision, Error, Proposal, Record, Snapshot, Status, hash, paths};

pub(crate) fn run(proposal: &Proposal, decision: Decision) -> Result<Record, ApplyError> {
    run_with(proposal, decision, write)
}

fn run_with(
    proposal: &Proposal,
    decision: Decision,
    mut apply_file: impl FnMut(&Snapshot, &paths::Target) -> Result<(), Error>,
) -> Result<Record, ApplyError> {
    let status = match decision {
        Decision::Approve => Status::Approved,
        Decision::Decline => Status::Declined,
    };
    let mut record = Record::new(proposal, status);
    let result = (|| {
        if decision == Decision::Decline {
            return Ok(());
        }
        let targets = preflight(proposal)?;
        for (file, target) in proposal.files.iter().zip(&targets) {
            apply_file(file, target)?;
            record.applied.push(file.path.clone());
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(record),
        Err(error) => {
            record.fail(&error);
            Err(ApplyError {
                error,
                record: Box::new(record),
            })
        }
    }
}

fn preflight(proposal: &Proposal) -> Result<Vec<paths::Target>, Error> {
    let root = paths::validate_root(&proposal.root)?;
    if root != proposal.canonical_root {
        return Err(Error::OutsideRoot {
            path: proposal.root.clone(),
        });
    }
    let mut targets = Vec::with_capacity(proposal.files.len());
    for file in &proposal.files {
        let target = paths::validate_target(&root, &file.path).map_err(|error| {
            // A missing parent also means a captured file disappeared. Keep this a
            // stale review rather than an application failure; Add still needs a parent.
            if file.base_hash.is_some()
                && matches!(&error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
            {
                Error::Stale { path: file.path.clone() }
            } else {
                error
            }
        })?;
        if target.metadata.is_some() != file.base_hash.is_some() {
            return Err(Error::Stale {
                path: file.path.clone(),
            });
        }
        let current_hash = match &target.metadata {
            None => None,
            Some(metadata) if metadata.is_file() => Some(hash(
                &fs::read(&target.absolute).map_err(|error| Error::io(&file.path, error))?,
            )),
            Some(_) => {
                return Err(Error::Stale {
                    path: file.path.clone(),
                });
            }
        };
        if current_hash != file.base_hash {
            return Err(Error::Stale {
                path: file.path.clone(),
            });
        }
        targets.push(target);
    }
    Ok(targets)
}

fn write(file: &Snapshot, target: &paths::Target) -> Result<(), Error> {
    let Some(content) = &file.content else {
        return fs::remove_file(&target.absolute).map_err(|error| Error::io(&file.path, error));
    };
    let parent = target.absolute.parent().ok_or_else(|| Error::InvalidPath {
        path: file.path.clone(),
    })?;
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|error| Error::io(&file.path, error))?;
    temporary
        .write_all(content)
        .map_err(|error| Error::io(&file.path, error))?;
    if let Some(permissions) = &file.permissions {
        temporary
            .as_file()
            .set_permissions(permissions.clone())
            .map_err(|error| Error::io(&file.path, error))?;
    }
    temporary
        .persist(&target.absolute)
        .map_err(|error| Error::io(&file.path, error.error))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io;

    use super::*;
    use crate::{Change, Request};

    #[test]
    fn write_error_reports_the_exact_applied_prefix_and_stops() {
        let root = tempfile::tempdir().unwrap();
        let proposal = Proposal::new(
            root.path(),
            Request {
                session_id: "session-1".into(),
                reason: "Add three profiles".into(),
                changes: ["first", "fails", "last"]
                    .into_iter()
                    .map(|path| Change::Add {
                        path: path.into(),
                        content: b"profile".to_vec(),
                    })
                    .collect(),
            },
        )
        .unwrap();
        let mut calls = Vec::new();
        let failure = run_with(&proposal, Decision::Approve, |file, target| {
            calls.push(file.path.clone());
            if file.path == std::path::Path::new("fails") {
                return Err(Error::io(
                    &file.path,
                    io::Error::other("injected write failure"),
                ));
            }
            write(file, target)
        })
        .unwrap_err();
        assert_eq!(
            calls,
            vec![std::path::PathBuf::from("first"), "fails".into()]
        );
        assert_eq!(failure.record.decision, Status::Failed);
        assert_eq!(
            failure.record.applied,
            vec![std::path::PathBuf::from("first")]
        );
        assert_eq!(
            failure.record.problem.as_ref().unwrap().path.as_deref(),
            Some(std::path::Path::new("fails"))
        );
        assert_eq!(fs::read(root.path().join("first")).unwrap(), b"profile");
        assert!(!root.path().join("fails").exists());
        assert!(!root.path().join("last").exists());
        let json = serde_json::to_string(&failure.record).unwrap();
        let restored: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, *failure.record);
    }
}
