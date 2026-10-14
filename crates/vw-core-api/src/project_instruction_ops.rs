//! Mutating project instruction operations, separate from read/list dispatch.
//! Depends on the saved project store, capability file helpers, refresh, and native confirmer.
//! Called only after the handler's user-only origin check and project-folder resolution.

use std::io::Write;
use std::path::Path;

use cap_std::fs::OpenOptions;
use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
use vw_projects::State;

use crate::command::{Origin, requester};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::project_instruction_files::{
    check_expected, check_name, check_text, checked_doc, file_dir, read_doc, write_existing,
};
use crate::project_instructions::{invalid, io_error, refuse};
use crate::project_instructions_view::{ProjectInstructionsRequest, ProjectInstructionsResult};

const DELETE_METHOD: &str = "vibe-work/projectInstructions/delete";

fn check_folder(folder: &Path, expected: &str) -> Result<(), CoreError> {
    if folder.to_string_lossy() == expected {
        Ok(())
    } else {
        Err(refuse(
            "changedFolder",
            "the project's main folder changed; reload and review the file",
        ))
    }
}

fn check_stored_folder(store: &State, project_id: &str, expected: &str) -> Result<(), CoreError> {
    if store
        .project(project_id)
        .and_then(|project| project.main_folder.as_deref())
        == Some(expected)
    {
        Ok(())
    } else {
        Err(refuse(
            "changedFolder",
            "the project's main folder changed; reload and review the file",
        ))
    }
}

/// Creates an absent file only after the explicit reviewed Save action.
pub(crate) async fn create(
    core: &Core,
    origin: Origin,
    folder: &Path,
    request: ProjectInstructionsRequest,
) -> Result<ProjectInstructionsResult, CoreError> {
    let ProjectInstructionsRequest::Create {
        project_id,
        name,
        expected_folder,
        text,
    } = request
    else {
        return Err(invalid("wrong project instruction operation"));
    };
    check_folder(folder, &expected_folder)?;
    check_text(&text)?;
    let store = core.shared.projects.store.clone();
    let task_folder = folder.to_path_buf();
    let task_name = name.clone();
    let doc = tokio::task::spawn_blocking(move || {
        store.read(|state| {
            check_stored_folder(state, &project_id, &expected_folder)?;
            checked_doc(&task_folder, &task_name, None)?;
            let (dir, leaf) = file_dir(&task_folder, &task_name, true)?;
            let mut file = dir
                .open_with(&leaf, OpenOptions::new().write(true).create_new(true))
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        refuse("alreadyExists", "the file now exists; reload and review it")
                    } else {
                        io_error(&error)
                    }
                })?;
            if let Err(error) = file
                .write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
            {
                let _ = dir.remove_file(&leaf);
                return Err(io_error(&error));
            }
            read_doc(&task_folder, &task_name)
        })
    })
    .await
    .map_err(|_| invalid("file creation was cancelled"))??;
    core.apply_project_instruction_folder(origin, &folder.to_string_lossy())
        .await?;
    Ok(ProjectInstructionsResult::Doc { name, doc })
}

/// Replaces an existing file only if the reviewed content still matches through the held directory.
pub(crate) async fn write(
    core: &Core,
    origin: Origin,
    folder: &Path,
    request: ProjectInstructionsRequest,
) -> Result<ProjectInstructionsResult, CoreError> {
    let ProjectInstructionsRequest::Write {
        project_id,
        name,
        expected_folder,
        text,
        expected,
    } = request
    else {
        return Err(invalid("wrong project instruction operation"));
    };
    check_folder(folder, &expected_folder)?;
    check_text(&text)?;
    let store = core.shared.projects.store.clone();
    let task_folder = folder.to_path_buf();
    let task_name = name.clone();
    let doc = tokio::task::spawn_blocking(move || {
        store.read(|state| {
            check_stored_folder(state, &project_id, &expected_folder)?;
            checked_doc(&task_folder, &task_name, Some(&expected))?;
            write_existing(&task_folder, &task_name, &text, &expected)?;
            read_doc(&task_folder, &task_name)
        })
    })
    .await
    .map_err(|_| invalid("file write was cancelled"))??;
    core.apply_project_instruction_folder(origin, &folder.to_string_lossy())
        .await?;
    Ok(ProjectInstructionsResult::Doc { name, doc })
}

/// Confirms the exact file path natively, then rechecks project folder and file content before removal.
pub(crate) async fn delete(
    core: &Core,
    origin: Origin,
    folder: &Path,
    request: ProjectInstructionsRequest,
) -> Result<ProjectInstructionsResult, CoreError> {
    let ProjectInstructionsRequest::Delete {
        project_id,
        name,
        expected_folder,
        expected,
    } = request
    else {
        return Err(invalid("wrong project instruction operation"));
    };
    check_folder(folder, &expected_folder)?;
    check_name(&name)?;
    let path = folder.join(&name);
    checked_doc(folder, &name, Some(&expected))?;
    let confirmation = ConfirmationRequest {
        method: DELETE_METHOD.to_owned(),
        requester: requester(origin),
        summary: format!(
            "Delete project instruction file {}?",
            vw_gate::dialog_quote(&path.to_string_lossy())
        ),
        params: json!({"path": path.to_string_lossy()}),
        always: None,
    };
    if core.shared.confirmer.confirm(&confirmation).await != Confirmation::Approved {
        return Err(CoreError::new(
            ErrorCode::ConfirmationDeclined,
            "the instruction file was kept",
        ));
    }
    let store = core.shared.projects.store.clone();
    let task_folder = folder.to_path_buf();
    let task_name = name.clone();
    tokio::task::spawn_blocking(move || {
        store.read(|state| {
            check_stored_folder(state, &project_id, &expected_folder)?;
            let (dir, leaf) = file_dir(&task_folder, &task_name, false)?;
            check_expected(&dir, &leaf, &expected)?;
            dir.remove_file(&leaf).map_err(|error| io_error(&error))
        })
    })
    .await
    .map_err(|_| invalid("file deletion was cancelled"))??;
    core.apply_project_instruction_folder(origin, &folder.to_string_lossy())
        .await?;
    Ok(ProjectInstructionsResult::Deleted { name })
}
