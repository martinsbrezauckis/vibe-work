//! Applies the shared file-read policy to each editor before panel serialization. Called after the window replies.
//! Uses the chat's current mode and folder layers; missing policy or an unresolved path withholds that editor.
use super::{PanelView, Shared, Value, agent_answer};
use crate::search_read_rules::ReadRules;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vw_agent_tools::folders::resolve;

const DENIED: &str =
    "Withheld by the person's file read rules. Do not read this file through another tool.";
const UNCHECKED: &str =
    "File read rules could not be checked. Do not read this file through another tool.";

pub(super) async fn answer(
    shared: &Shared,
    session: &str,
    view: PanelView,
) -> Result<Value, String> {
    let home = shared.instructions.own_home.clone();
    let roots = crate::agent_tools::session::folders(shared, session);
    let mode = shared.access.remembered(session);
    tokio::task::spawn_blocking(move || {
        let policy = roots.and_then(|roots| {
            let mode = mode.as_deref().ok_or(UNCHECKED)?;
            let root = roots.first().ok_or(UNCHECKED)?.clone();
            let rules = ReadRules::load_for_mode(
                &home,
                &roots,
                mode,
                Instant::now() + Duration::from_secs(2),
            )?;
            Ok((rules, root))
        });
        agent_answer(view, |path| reason(&policy, path))
    })
    .await
    .map_err(|_| UNCHECKED.to_owned())
}

fn reason(policy: &Result<(ReadRules, PathBuf), String>, path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return None;
    }
    let Ok((rules, root)) = policy else {
        return Some(UNCHECKED);
    };
    // Absolute editors may be outside the chat folder. Relative editors use its cwd, never the process cwd.
    let path = root.join(path);
    if !rules.allows(&path) {
        return Some(DENIED);
    }
    // Check aliases too, as search does. Resolution also handles unsaved files that do not exist on disk yet.
    match resolve(&path) {
        Ok(real) if rules.allows(&real) => None,
        Ok(_) => Some(DENIED),
        Err(_) => Some(UNCHECKED),
    }
}
