//! Safety reminders on the core side: the `safety` command (Settings > Safety and the
//! Debug API, acting as the user: no dialog) and the check the core runs itself before `process_start` and
//! `secret_run` (Vibe's `pre_tool` hook never sees those; it runs the same rules for `bash`).
//!
//! The settings live in `safety.json` next to the hook endpoint file in the agent tools' user-only data folder
//! (vw-agent-tools `safety::settings_file`), written atomically; the hook reads them on every call, so a change
//! applies to the next command without restarting anything.
//!
//! Depends on: vw-agent-tools (`safety`), vw-platform (private files), `safety_view`. Called by: `dispatch` (the
//! `safety` command), `agent_tools` (`tools`, `secret_run`).
//! Invariant: without agent tools nothing is checked (`active: false`); the settings are then neither read nor
//! written.

use std::path::PathBuf;

use vw_agent_tools::safety::{self, CustomRule, SafetySettings};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::safety_view::{CustomRuleView, SafetyRequest, SafetyResult, SafetyRuleView};

fn path(shared: &Shared) -> Option<PathBuf> {
    shared
        .config
        .agent_tools
        .as_ref()
        .map(|c| safety::settings_file(&c.endpoint_file()))
}

/// The core's own check before it starts a command for the agent. `Err` = the refusal for the agent.
///
/// # Errors
/// The safety reminder's text.
pub(crate) fn guard(
    shared: &Shared,
    command: &str,
    cwd: Option<&std::path::Path>,
) -> Result<(), String> {
    let Some(path) = path(shared) else {
        return Ok(());
    };
    match safety::check(command, cwd, &safety::load(&path)) {
        Some(stop) => {
            tracing::info!(rule = %stop.rule, "safety reminder stopped an agent command");
            Err(stop.text)
        }
        None => Ok(()),
    }
}

fn view(settings: &SafetySettings, active: bool) -> SafetyResult {
    SafetyResult {
        active,
        rules: safety::RULES
            .iter()
            .map(|r| SafetyRuleView {
                id: r.id.to_owned(),
                label: r.label.to_owned(),
                on: !settings.off.iter().any(|o| o == r.id),
            })
            .collect(),
        custom: settings
            .custom
            .iter()
            .map(|c| CustomRuleView {
                id: c.id.clone(),
                text: c.text.clone(),
            })
            .collect(),
    }
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}

/// Executes one `safety` request.
///
/// # Errors
/// [`ErrorCode::InvalidCommand`] (unknown rule, bad text, too many rules), [`ErrorCode::Internal`] when the
/// settings cannot be written.
pub(crate) fn execute(shared: &Shared, request: SafetyRequest) -> Result<SafetyResult, CoreError> {
    let Some(path) = path(shared) else {
        return Ok(view(&SafetySettings::default(), false));
    };
    let mut settings = safety::load(&path);
    match request {
        SafetyRequest::Read {} => return Ok(view(&settings, true)),
        SafetyRequest::SetRule { id, on } => {
            if !safety::RULES.iter().any(|r| r.id == id) {
                return Err(invalid("no such safety rule"));
            }
            settings.off.retain(|o| *o != id);
            if !on {
                settings.off.push(id);
            }
        }
        SafetyRequest::AddCustom { text } => {
            let text = text.trim().to_owned();
            if text.is_empty()
                || text.chars().count() > safety::MAX_CUSTOM_TEXT
                || text.contains(['\n', '\r', '\0'])
            {
                return Err(invalid("the text must be 1 to 200 characters on one line"));
            }
            if settings.custom.len() >= safety::MAX_CUSTOM {
                return Err(invalid("at most 100 rules of your own"));
            }
            if !settings
                .custom
                .iter()
                .any(|c| c.text.eq_ignore_ascii_case(&text))
            {
                let id = vw_agent_tools::credential::new_credential()
                    .map(|c| c[..12].to_owned())
                    .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
                settings.custom.push(CustomRule { id, text });
            }
        }
        SafetyRequest::RemoveCustom { id } => {
            let before = settings.custom.len();
            settings.custom.retain(|c| c.id != id);
            if settings.custom.len() == before {
                return Err(invalid("no such rule of your own"));
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?;
    vw_platform::replace_private(&path, &bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("the safety settings could not be saved: {e}"),
        )
    })?;
    Ok(view(&settings, true))
}
