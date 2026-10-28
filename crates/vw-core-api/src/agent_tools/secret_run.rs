//! Vault broker tools: list visible metadata and run one program with named secrets in its environment.
//! Depends on current vault policy, native consent and process guards; called by tool dispatch.
//! Values go only to the child's environment; tool output passes the registered-secret scrubber.
//! Permission holds in the hook; running commands retain their job result for later status reads.

mod consent;
mod status;
use status::wait;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::watch;
use vw_agent_tools::secretspec::{self, RunSpec};
use vw_agent_tools::wire::tools as names;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_keys::ProjectSecret;
use vw_vault::SecretString;

use super::AgentTools;
use super::session::{folders, inside};
use crate::core::Shared;
use crate::secrets_view::SecretUseView;

/// Longest wait of one tool call for the result (under Vibe's 60 s program budget and the binary's 50 s).
pub(crate) const WAIT: Duration = Duration::from_secs(25);
/// Running commands per session.
pub(crate) const MAX_OPEN: usize = 8;
/// How long a result or its already-reported receipt is kept after completion.
const KEEP: Duration = Duration::from_secs(15 * 60);
/// The dialog method name.
pub(crate) const METHOD: &str = "vibe-work/secretRun";
#[derive(Clone, Debug)]
enum RunState {
    Running,
    Finished(Value, Instant),
    Refused(String, Instant),
    Reported {
        outcome: &'static str,
        at: String,
        finished: Instant,
    },
}

struct Run {
    session_id: String,
    state: watch::Receiver<RunState>,
    /// The call already answered "running"; only then does the finish wake the agent.
    handed_off: bool,
}

/// The open and finished requests of this run.
#[derive(Default)]
pub(crate) struct SecretRuns {
    runs: Mutex<HashMap<String, Run>>,
}

impl SecretRuns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Run>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn handed_off(&self, id: &str) -> bool {
        self.lock().get(id).is_some_and(|run| run.handed_off)
    }

    /// Drops results older than [`KEEP`], then counts the session's open requests.
    fn open_of(&self, session_id: &str) -> usize {
        let mut runs = self.lock();
        runs.retain(|_, r| match &*r.state.borrow() {
            RunState::Finished(_, finished)
            | RunState::Refused(_, finished)
            | RunState::Reported { finished, .. } => finished.elapsed() < KEEP,
            RunState::Running => true,
        });
        runs.values()
            .filter(|r| r.session_id == session_id && !terminal(&r.state.borrow()))
            .count()
    }
}

fn terminal(state: &RunState) -> bool {
    matches!(
        state,
        RunState::Finished(..) | RunState::Refused(..) | RunState::Reported { .. }
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Runs one secret tool for `session_id`.
pub(crate) async fn call(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    crate::agent_access::check_for(shared, "vibe-work/secretRun", session_id)?;
    if shared.store.view(session_id).is_none() {
        return Err("this session is not open in Vibe Work right now".to_owned());
    }
    match tool {
        names::SECRET_LIST => list(shared, session_id).await,
        names::SECRET_RUN => Err("This secret command needs its pre-tool hook decision.".into()),
        names::SECRET_RUN_STATUS => {
            let id = args
                .get("requestId")
                .and_then(Value::as_str)
                .ok_or("requestId is required")?;
            wait(tools, session_id, id).await
        }
        _ => Err("unknown tool".to_owned()),
    }
}

fn keys(shared: &Shared) -> Result<&Arc<vw_keys::KeyService>, String> {
    shared
        .config
        .keys
        .as_ref()
        .ok_or_else(|| "Vibe Work has no vault connected, so no secret can be used".to_owned())
}

async fn secrets(shared: &Shared) -> Result<Vec<ProjectSecret>, String> {
    keys(shared)?.project_secrets().await.map_err(|e| {
        if vw_keys::is_locked(&e) {
            "the user's vault is locked: ask the user to unlock it in Vibe Work (Settings > Account), then try \
             again"
                .to_owned()
        } else {
            format!("the vault could not be read ({e})")
        }
    })
}

/// Hide User only entries before name/id resolution, using the current project policy.
async fn visible_secrets(shared: &Shared, session_id: &str) -> Result<Vec<ProjectSecret>, String> {
    let entries = secrets(shared).await?;
    let project = permission_project_of(shared, session_id, &folders(shared, session_id)?)?;
    let mut visible = Vec::new();
    for secret in entries {
        if project
            .policy(keys(shared)?, &secret.id)
            .await
            .map_err(|_| "the entry permission could not be checked".to_owned())?
            .permission
            != vw_keys::EntryPermission::Never
        {
            visible.push(secret);
        }
    }
    Ok(visible)
}

pub(crate) async fn visible_logins(
    shared: &Shared,
    project: &crate::entry_permissions::Identity,
) -> Result<Vec<vw_keys::Login>, String> {
    let keys = keys(shared)?;
    let mut visible = Vec::new();
    let logins = keys.logins().await.map_err(|e| {
        if vw_keys::is_locked(&e) {
            "the user's vault is locked: ask the user to unlock it in Vibe Work (Settings > Account)".to_owned()
        } else {
            "the vault could not be read".to_owned()
        }
    })?;
    for login in logins {
        let policy = project
            .policy(keys, &login.id)
            .await
            .map_err(|_| "the entry permission could not be checked".to_owned())?;
        if policy.permission != vw_keys::EntryPermission::Never {
            visible.push(login);
        }
    }
    Ok(visible)
}

/// The session-open project identity; no Git metadata or live session folder can broaden it.
pub(crate) fn permission_project_of(
    shared: &Shared,
    session_id: &str,
    folders: &[PathBuf],
) -> Result<crate::entry_permissions::Identity, String> {
    let identity = shared.entry_projects.get(session_id)?;
    if folders
        .first()
        .is_none_or(|folder| *folder != Path::new(&identity.plain))
    {
        return Err(
            "this session's folder no longer matches its captured project identity".to_owned(),
        );
    }
    Ok(identity)
}

async fn list(shared: &Arc<Shared>, session_id: &str) -> Result<Value, String> {
    let secrets = visible_secrets(shared, session_id).await?;
    let project = permission_project_of(shared, session_id, &folders(shared, session_id)?)?;
    let logins: Vec<Value> = visible_logins(shared, &project)
        .await?
        .into_iter()
        .filter(|l| l.has_password)
        .map(|l| json!({"name": l.name, "site": l.origin, "username": l.username, "id": l.id}))
        .collect();
    Ok(json!({
        "logins": logins,
        "howToSignIn": "browser_act {action: \"fill\", ref: <a password or user-name field from browser_read>, login: \
                        <name>}",
        "secrets": secrets.iter().map(|s| json!({"id": s.id, "name": s.name, "fields": s.fields})).collect::<Vec<_>>(),
        "howToUse": "Call secret_run with program, args and env [{\"name\": \"VARIABLE\", \"secret\": \"<id or name>\"}]",

    }))
}

/// One binding resolved to a secret.
struct Bound {
    env: String,
    secret: ProjectSecret,
    field: Option<String>,
}

fn bind(spec: &RunSpec, secrets: &[ProjectSecret]) -> Result<Vec<Bound>, String> {
    spec.bindings
        .iter()
        .map(|b| {
            let wanted = b.secret.to_lowercase();
            let secret = secrets
                .iter()
                .find(|s| s.id == b.secret)
                .or_else(|| secrets.iter().find(|s| s.name.to_lowercase() == wanted))
                .cloned()
                .ok_or_else(|| missing_secret(&b.secret))?;
            if let Some(field) = &b.field
                && !secret.fields.contains(field)
            {
                return Err(format!(
                    "the secret '{}' has no field '{}' (it has: {})",
                    secret.name,
                    field.chars().take(40).collect::<String>(),
                    secret.fields.join(", ")
                ));
            }
            Ok(Bound {
                env: b.env.clone(),
                secret,
                field: b.field.clone(),
            })
        })
        .collect()
}

fn missing_secret(name: &str) -> String {
    format!(
        "no secret named '{}'; ask the user to add it in Vibe Work (Settings > Project secrets)",
        name.chars().take(60).collect::<String>()
    )
}

/// A plain shell-like rendering of the command, for the guards and the log (arguments with spaces quoted).
fn command_text(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|w| {
            if w.is_empty()
                || w.contains(|c: char| c.is_whitespace() || "'\"\\$`;&|<>()".contains(c))
            {
                format!("'{}'", w.replace('\'', "'\\''"))
            } else {
                w.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn path_var(
    env: &[(std::ffi::OsString, std::ffi::OsString)],
    name: &str,
) -> Option<std::ffi::OsString> {
    env.iter()
        .find(|(k, _)| k.to_string_lossy().eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

/// Vibe's card for this exact `secret_run` call (program, arguments, which secret goes where) came before this call;
/// the user's yes there answers the git and kill guards' "may this command run" question only. It never stands in
/// for the consent dialog (giving a secret is another question, native dialog only).
fn card_said_yes(shared: &Shared, session_id: &str, args: &Value) -> bool {
    crate::one_ask::of(shared).is_some_and(|ledger| {
        ledger.card_said_yes(
            session_id,
            &format!(
                "mcp_{}.{}",
                vw_agent_tools::wire::SERVER_NAME,
                names::SECRET_RUN
            ),
            args,
        )
    })
}

/// Checks the request and holds native consent before preparing one command execution.
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    args: &Value,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    crate::agent_access::check_for(shared, METHOD, session_id)?;
    let spec = secretspec::parse(args)?;
    if tools.secret_runs.open_of(session_id) >= MAX_OPEN {
        return Err(format!(
            "this session already has {MAX_OPEN} secret_run commands running; collect them with \
             secret_run_status first"
        ));
    }
    let secrets = visible_secrets(shared, session_id).await?;
    let bound = bind(&spec, &secrets)?;
    let folders = folders(shared, session_id)?;
    let cwd =
        crate::guard_refusals::folder(shared, session_id, inside(&folders, spec.cwd.as_deref()))?;
    let policy_project = permission_project_of(shared, session_id, &folders)?;
    let project = policy_project.allowed_project().to_owned();
    #[cfg(windows)]
    let env = super::secret_env::for_run(shared, tools, &cwd).await?;
    #[cfg(not(windows))]
    let env = tools.config.process_env.clone();
    let program = secretspec::resolve_program(
        &spec.program,
        &cwd,
        path_var(&env, "PATH").as_deref(),
        path_var(&env, "PATHEXT").as_deref(),
    )
    .ok_or_else(|| {
        format!(
            "program '{}' was not found (not on PATH and not a file in the folder)",
            spec.program.chars().take(80).collect::<String>()
        )
    })?;
    let program = vw_platform::canonical_plain(&program)
        .map_err(|_| "the program could not be resolved".to_owned())?;
    let program_text = program.to_string_lossy().into_owned();
    // The real program may be an interpreter under another name (a link): checked by its real name.
    if let Some(reason) = secretspec::inline_evaluation(&program_text, &spec.args) {
        return Err(reason);
    }
    let command = command_text(&program_text, &spec.args);
    let asked = crate::one_ask::Asked::Card(card_said_yes(shared, session_id, args));
    super::session::check_config_command(shared, &cwd, &command)?;
    let found = vw_agent_tools::killparse::scan(&command);
    super::killcheck::guard(shared, session_id, &command, &found, asked).await??;
    super::gitcheck::guard(shared, session_id, &cwd, &command, asked).await?;
    crate::safety::guard(shared, session_id, &command, Some(&cwd))?;
    let place = Place {
        project: &project,
        cwd: &cwd,
    };
    let dialog = dialog_request(shared, session_id, &spec, &bound, &place);
    let id = vw_agent_tools::credential::new_credential()
        .map(|c| format!("secret-run-{}", &c[..16]))
        .map_err(|e| format!("no request id could be made: {e}"))?;
    let mut dialog = dialog;
    dialog.params["askId"] = Value::String(id.clone());
    let job = Job {
        shared: Arc::downgrade(shared),
        session_id: session_id.to_owned(),
        spec,
        bound,
        program,
        cwd,
        project,
        policy_project,
        dialog,
        env,
    };
    let policies = job.permission().await?;
    let tools = Arc::downgrade(tools);
    let session_id = session_id.to_owned();
    Ok(Box::pin(async move {
        let tools = tools.upgrade().ok_or("Tools closed.")?;
        let (sender, receiver) = watch::channel(RunState::Running);
        tools.secret_runs.lock().insert(
            id.clone(),
            Run {
                session_id: session_id.clone(),
                state: receiver,
                handed_off: false,
            },
        );
        spawn_job(job, policies, sender, id.clone());
        wait(&tools, &session_id, &id).await
    }))
}

fn spawn_job(
    job: Job,
    policies: consent::Permission,
    sender: watch::Sender<RunState>,
    finished_id: String,
) {
    tokio::spawn(async move {
        let Some(owner) = job.shared.upgrade() else {
            return;
        };
        let session = job.session_id.clone();
        let outcome = job.run(policies).await;
        let _ = sender.send(match outcome {
            Ok(result) => RunState::Finished(result, Instant::now()),
            Err(reason) => RunState::Refused(reason, Instant::now()),
        });
        let handed_off = owner
            .agent_tools
            .as_ref()
            .is_some_and(|tools| tools.secret_runs.handed_off(&finished_id));
        if handed_off {
            let _ = crate::ask::continuation::secret_finished(owner, session, finished_id).await;
        }
    });
}

/// Where a request runs: the session's project, its copy of the project (if any) and the working folder.
struct Place<'a> {
    project: &'a str,
    cwd: &'a Path,
}

fn dialog_request(
    shared: &Shared,
    session_id: &str,
    spec: &RunSpec,
    bound: &[Bound],
    place: &Place<'_>,
) -> ConfirmationRequest {
    let title = shared.names.confirmation_title(&shared.store, session_id);
    let names: Vec<String> = bound.iter().map(|b| b.secret.name.clone()).collect();
    ConfirmationRequest {
        method: METHOD.to_owned(),
        requester: Requester::Agent,
        summary: super::secret_dialog::summary(
            &title,
            place.project,
            place.cwd,
            &names,
            &spec.program,
            &spec.args,
        ),
        params: json!({"program": spec.program, "args": spec.args, "folder": place.cwd,
            "secrets": names, "project": place.project, "session": title}),
        always: Some(vw_vocabulary::label("vaultPermissions", "allowed").to_owned()),
    }
}

#[derive(Clone, Copy)]
enum Approval {
    EntryAllowed,
    Once,
    Always,
}
impl Approval {
    fn as_str(self) -> &'static str {
        match self {
            Self::EntryAllowed => "entryAllowed",
            Self::Once => "allowedOnce",
            Self::Always => "allowedAlways",
        }
    }
}

struct Job {
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    shared: std::sync::Weak<Shared>,
    session_id: String,
    spec: RunSpec,
    bound: Vec<Bound>,
    program: PathBuf,
    cwd: PathBuf,
    project: String,
    policy_project: crate::entry_permissions::Identity,
    dialog: ConfirmationRequest,
}
