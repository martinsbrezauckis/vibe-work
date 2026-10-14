//! Checks a session's copy folder before adopting its opened state. Depends on JSON and the folder resolver;
//! called by sessions. The cwd must remain inside the copy recorded by the session.

use std::path::Path;

use serde_json::Value;
use vw_agent_tools::folders::{Layout, resolve};

pub(crate) fn check(home: &Path, state: &Value, previous: Option<&Value>) -> Result<(), String> {
    let Some(cwd) = state
        .pointer("/session/cwd")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    Layout::new(home, Path::new(cwd)).map_err(|e| e.to_string())?;
    let copy = previous
        .and_then(|v| v.pointer("/session/worktree/path"))
        .and_then(Value::as_str)
        .or_else(|| {
            state
                .pointer("/session/worktree/path")
                .and_then(Value::as_str)
        });
    if let Some(copy) = copy {
        if !Path::new(copy).is_absolute() || !Path::new(cwd).is_absolute() {
            return Err("session copy and cwd must be absolute".to_owned());
        }
        let root = resolve(Path::new(copy)).map_err(|e| e.to_string())?;
        let here = resolve(Path::new(cwd)).map_err(|e| e.to_string())?;
        if !here.starts_with(&root) {
            return Err(format!(
                "session folder {cwd} resolves outside its copy {copy}"
            ));
        }
    }
    Ok(())
}
