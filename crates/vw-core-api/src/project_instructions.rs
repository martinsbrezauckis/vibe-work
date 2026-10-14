//! Project instruction files in saved main folders.
//! Depends on the project store, document shape and session refresh, and the native confirmer;
//! called by `Command::ProjectInstructions` from the Projects screen.
//! Invariants: the client supplies only a project ID and an allowlisted relative name; links and
//! stale folder or content snapshots are refused; deletion always asks the native confirmer.

use std::path::{Path, PathBuf};

use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::project_instruction_files::{check_name, file_dir, open_root, read_doc};
use crate::project_instruction_links::{linked_from_agents, unlink_draft};
use crate::project_instructions_view::{
    ProjectInstructionFile, ProjectInstructionsRequest, ProjectInstructionsResult,
};

pub(crate) fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

pub(crate) fn refuse(code: &str, message: impl Into<String>) -> CoreError {
    let mut error = CoreError::new(ErrorCode::Denied, message);
    error.server_code = Some(code.to_owned());
    error
}

pub(crate) fn io_error(error: &std::io::Error) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("project instructions: {}", error.kind()),
    )
}

fn list(folder: &Path) -> Result<Vec<ProjectInstructionFile>, CoreError> {
    let mut names = vec!["AGENTS.md".to_owned()];
    let root = read_doc(folder, "AGENTS.md")?;
    // A missing directory is fine, but a linked one is a refusal, including for listing.
    match file_dir(folder, "docs/instructions/sample.md", false) {
        Ok((dir, _)) => {
            for entry in dir.entries().map_err(|error| io_error(&error))? {
                let entry = entry.map_err(|error| io_error(&error))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let candidate = format!("docs/instructions/{name}");
                if check_name(&candidate).is_ok() {
                    names.push(candidate);
                }
            }
        }
        Err(error) if error.code == ErrorCode::NotFound => {}
        Err(error) => return Err(error),
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let doc = read_doc(folder, &name)?;
            Ok(ProjectInstructionFile {
                exists: doc.exists,
                linked: name == "AGENTS.md" || linked_from_agents(&root.text, &name),
                editable: doc.editable,
                name,
            })
        })
        .collect()
}

impl Core {
    fn project_instruction_folder(&self, project_id: &str) -> Result<PathBuf, CoreError> {
        let folder = self
            .shared
            .projects
            .store
            .read(|store| {
                store
                    .project(project_id)
                    .and_then(|project| project.main_folder.clone())
            })
            .ok_or_else(|| {
                CoreError::new(ErrorCode::NotFound, "project has no saved main folder")
            })?;
        let path = PathBuf::from(folder);
        open_root(&path)?;
        Ok(path)
    }

    /// Executes an explicit project instruction file action.
    pub(crate) async fn project_instructions(
        &self,
        origin: Origin,
        request: ProjectInstructionsRequest,
    ) -> Result<CommandResult, CoreError> {
        if origin != Origin::Ui {
            return Err(refuse(
                "projectInstructionsUserOnly",
                "project instruction files can only be managed in the app window",
            ));
        }
        let project_id = match &request {
            ProjectInstructionsRequest::List { project_id }
            | ProjectInstructionsRequest::Read { project_id, .. }
            | ProjectInstructionsRequest::LinkDraft { project_id, .. }
            | ProjectInstructionsRequest::Create { project_id, .. }
            | ProjectInstructionsRequest::Write { project_id, .. }
            | ProjectInstructionsRequest::Delete { project_id, .. } => project_id,
        };
        let folder = self.project_instruction_folder(project_id)?;
        let result = match request {
            ProjectInstructionsRequest::List { .. } => {
                let folder_text = folder.to_string_lossy().into_owned();
                let files = tokio::task::spawn_blocking(move || list(&folder))
                    .await
                    .map_err(|_| invalid("file listing was cancelled"))??;
                ProjectInstructionsResult::List {
                    folder: folder_text,
                    files,
                }
            }
            ProjectInstructionsRequest::Read { name, .. } => {
                let doc = tokio::task::spawn_blocking({
                    let folder = folder.clone();
                    let name = name.clone();
                    move || read_doc(&folder, &name)
                })
                .await
                .map_err(|_| invalid("file read was cancelled"))??;
                ProjectInstructionsResult::Doc { name, doc }
            }
            ProjectInstructionsRequest::LinkDraft { name, linked, .. } => {
                if name == "AGENTS.md" {
                    return Err(invalid("AGENTS.md cannot link to itself"));
                }
                check_name(&name)?;
                if linked {
                    let extra = read_doc(&folder, &name)?;
                    if !extra.exists || !extra.editable {
                        return Err(refuse(
                            "notEditable",
                            "the linked instruction file must exist and be readable",
                        ));
                    }
                }
                let doc = read_doc(&folder, "AGENTS.md")?;
                if !doc.editable {
                    return Err(refuse("notEditable", "AGENTS.md cannot be edited here"));
                }
                let label = name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&name)
                    .trim_end_matches(".md");
                let link = format!("[{label}]({name})");
                let draft = if linked {
                    if linked_from_agents(&doc.text, &name) {
                        doc.text.clone()
                    } else if doc.text.is_empty() {
                        format!("{link}\n")
                    } else {
                        format!(
                            "{}{}{link}\n",
                            doc.text,
                            if doc.text.ends_with('\n') { "" } else { "\n" }
                        )
                    }
                } else {
                    unlink_draft(&doc.text, &name)
                };
                ProjectInstructionsResult::LinkDraft { doc, draft }
            }
            request @ ProjectInstructionsRequest::Create { .. } => {
                crate::project_instruction_ops::create(self, origin, &folder, request).await?
            }
            request @ ProjectInstructionsRequest::Write { .. } => {
                crate::project_instruction_ops::write(self, origin, &folder, request).await?
            }
            request @ ProjectInstructionsRequest::Delete { .. } => {
                crate::project_instruction_ops::delete(self, origin, &folder, request).await?
            }
        };
        Ok(CommandResult::ProjectInstructions { result })
    }
}
