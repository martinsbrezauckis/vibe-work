//! App-owned push preparation and execution; the authenticated hook owns one exact plan and consent.
//! Uses the existing Git planner/runner, chat cards and notices; never passes credentials to a tool process.
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::Core;
use crate::ask::tool_calls::receipts::Execution;
use crate::core::Shared;

const OFF: &str = vw_vocabulary::texts::DEVELOPER_PERMISSIONS_OFF;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    branch: Option<String>,
    remote: Option<String>,
    push_id: Option<String>,
}

pub(super) fn unprepared(shared: &Shared) -> Result<Value, String> {
    check_enabled(shared)?;
    Err(
        "The push tool requires its own authenticated pre-tool check; request a new push call."
            .into(),
    )
}

fn check_enabled(shared: &Shared) -> Result<(), String> {
    if shared.developer_permissions.enabled() {
        Ok(())
    } else {
        Err(OFF.into())
    }
}

pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: &Value,
) -> Result<Execution, String> {
    if shared.store.view(owner).is_none() {
        return Err(vw_vocabulary::texts::PUSH_CHAT_UNAVAILABLE.into());
    }
    for key in ["branch", "remote", "pushId"] {
        if let Some(value) = args.get(key)
            && value
                .as_str()
                .is_none_or(|value| value.is_empty() || value.len() > 255)
        {
            return Err(format!(
                "{key} must be a nonempty string of at most 255 bytes."
            ));
        }
    }
    let input: Input =
        serde_json::from_value(args.clone()).map_err(|e| format!("Invalid push request: {e}"))?;
    if let Some(push_id) = input.push_id {
        if input.branch.is_some() || input.remote.is_some() || push_id.is_empty() {
            return Err("Supply only pushId when collecting a push result.".into());
        }
        let tools = shared
            .agent_tools
            .as_ref()
            .ok_or("Tools unavailable.")?
            .clone();
        let owner = owner.to_owned();
        return Ok(Box::pin(async move {
            tools
                .pushes
                .result(&owner, &push_id, Duration::from_secs(40))
                .await
        }));
    }
    check_enabled(shared)?;
    let core = Core {
        shared: shared.clone(),
    };
    let place = core.git_place(owner).map_err(|e| e.message)?;
    let planning = place.clone();
    let plan = crate::worktrees::blocking(move || {
        let state =
            vw_git::state::read_state(&planning.root, &planning.git_dir, planning.base.as_deref())?;
        if let Some(operation) = state.in_progress {
            return Err(vw_git::GitError::Blocked(format!(
                "Git has {} in progress here.",
                operation.words()
            )));
        }
        vw_git::push::plan_local_branch(
            &planning.root,
            &state,
            input.remote.as_deref(),
            input.branch.as_deref(),
        )
    })
    .await
    .map_err(|e| e.message)?
    .map_err(|e| e.to_string())?;
    let full = shared.access.remembered(owner).as_deref() == Some("auto-approve");
    if !full {
        let detail = json!({"kind":"approval", "choices":["approve","deny"], "requiredPermissions":[],
            "effect":{"kind":"tool", "toolName":"mcp_vw.push", "input":args,
                "display":{"summary":crate::git_publish::push_consent(&plan),
                    "content":format!("{}\nRemote: {}\nAddress: {}\nBranch: {}\nCommit: {}", place.root.display(), plan.remote, plan.url, plan.branch, plan.commit)},
                "vwPushPlan":plan}});
        crate::ask::tool_calls::approve(shared, owner, source, call, detail).await?;
    }
    let shared = shared.clone();
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let tools = shared
            .agent_tools
            .as_ref()
            .ok_or("Tools unavailable.")?
            .clone();
        let facts = json!({"remote":plan.remote, "address":plan.url, "branch":plan.branch,
            "remoteBranch":plan.remote_branch, "commit":plan.commit, "commits":plan.commits});
        let id = tools.pushes.start(&owner, shared.asks.now_ms(), facts)?;
        let (running, session, push_id) = (shared.clone(), owner.clone(), id.clone());
        tokio::spawn(async move {
            let outcome = execute(&running, &session, full, place, plan).await;
            if let Some(tools) = &running.agent_tools
                && let Err(error) = tools.pushes.finish(&push_id, outcome)
            {
                tracing::warn!(%error, "push outcome could not be saved");
            }
            let current = running
                .agent_tools
                .as_ref()
                .and_then(|tools| tools.pushes.session(&push_id))
                .unwrap_or(session);
            crate::ask::tool_calls::changed(&running, &current);
        });
        tools
            .pushes
            .result(&owner, &id, Duration::from_secs(40))
            .await
    }))
}

async fn execute(
    shared: &Arc<Shared>,
    owner: &str,
    full: bool,
    place: crate::git_cmd::Place,
    plan: vw_git::push::PushPlan,
) -> Result<Value, String> {
    let _one = shared.git.lock.lock().await;
    check_enabled(shared)?;
    if full && shared.access.remembered(owner).as_deref() != Some("auto-approve") {
        return Err("The chat's access mode changed. Request a new push for approval.".into());
    }
    let core = Core {
        shared: shared.clone(),
    };
    core.git_validate_place(owner, &place)
        .map_err(|e| e.message)?;
    let root = place.root.clone();
    let pushing = plan.clone();
    let summary =
        crate::worktrees::blocking(move || vw_git::push::push_local_branch(&root, &pushing))
            .await
            .map_err(|e| e.message)?
            .map_err(|e| e.to_string())?;
    Ok(
        json!({"status":"pushed", "commit":plan.commit, "remote":plan.remote,
        "remoteBranch":plan.remote_branch, "branch":plan.branch, "address":plan.url,
        "message":notice(&plan), "summary":summary}),
    )
}

fn notice(plan: &vw_git::push::PushPlan) -> String {
    let template = vw_vocabulary::texts::AGENT_PUSH_NOTICE;
    let mut remaining = template;
    let mut text = String::new();
    while let Some(start) = remaining.find('{') {
        text.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find('}').map(|end| start + end) else {
            break;
        };
        let value = match &remaining[start + 1..end] {
            "branch" => &plan.branch,
            "remote" => &plan.remote,
            "remoteBranch" => &plan.remote_branch,
            "commit" => &plan.commit,
            _ => &remaining[start..=end],
        };
        text.push_str(value);
        remaining = &remaining[end + 1..];
    }
    text.push_str(remaining);
    text
}
