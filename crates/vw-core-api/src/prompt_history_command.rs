//! Typed prompt-history handler. Uses the CLI home resolved at startup and the title vault-value guard;
//! callers cannot select a file or read secrets by writing a prompt.
use crate::core::Shared;
use crate::{CommandResult, CoreError, ErrorCode};
use std::sync::Arc;
pub(crate) async fn execute(
    shared: &Arc<Shared>,
    text: Option<String>,
) -> Result<CommandResult, CoreError> {
    let Some(home) = shared.instructions.cli_home.clone() else {
        return Ok(CommandResult::PromptHistory {
            entries: Vec::new(),
        });
    };
    crate::title_guard::refresh(shared).await;
    let shared = shared.clone();
    tokio::task::spawn_blocking(move || {
        let path = home.join("vibehistory");
        if let Some(text) = text {
            crate::prompt_history::append(&path, &text, &|s| !shared.titles.contains(s))?;
        }
        let entries = crate::prompt_history::read(&path)?
            .into_iter()
            .filter(|s| !shared.titles.contains(s))
            .collect();
        Ok(CommandResult::PromptHistory { entries })
    })
    .await
    .map_err(|_| {
        CoreError::new(
            ErrorCode::Internal,
            "Prompt history could not be read or saved. Try again.",
        )
    })?
    .map_err(|_: std::io::Error| {
        CoreError::new(
            ErrorCode::Internal,
            "Prompt history could not be read or saved. Try again.",
        )
    })
}
