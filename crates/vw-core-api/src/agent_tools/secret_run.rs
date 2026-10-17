//! The secret broker's agent tools: `secret_list`,
//! `secret_run`, `secret_run_status`. ShellX Vault's `InjectEnv` flow (`tauri-plugin-vault/src/commands.rs:449-715`
//! @ ded9de1) as Vibe Work's own code: the agent names a program, its arguments and which secret goes into which
//! variable; the user sees exactly that in a native dialog and allows it once, always in this project, or not; the
//! core reads the values from the vault, starts the ONE program with them in its environment, and answers its exit
//! code and output. The agent never gets a value.
//!
//! - **Consent:** every binding already covered by an "always" grant of this project -> no dialog (counted as a
//!   use); otherwise one dialog for the whole request (requester `Agent`: the window is asked even while the Debug
//!   API is on). "Always for this command" stores a grant of this exact pinned command in the vault (`secret_grants`,
//!   never a file of ours, read from the vault for every decision).
//! - **Same guards as `process_start`:** the kill guard, the git guard and the safety reminders read the command.
//! - **Running** (`secret_exec`): the agent process allowlist (`AgentToolsConfig::process_env`) plus the bound
//!   variables; stdin closed; Unix: its own process group, Windows: no console window; registered with the process
//!   monitor as this session's agent process, stopped (with its tree) when it runs past its time limit.
//! - **Output:** at most `secret_exec::OUTPUT_CAP` bytes per stream. Values are withheld by the agent tools listener's scrub
//!   (`server.rs`, every registered secret in every searched form), which every answer passes.
//! - **Timing:** Vibe gives an MCP call 60 s; a call waits at most [`WAIT`] and otherwise answers a `requestId`
//!   (`secret_run_status` waits again). At most [`MAX_OPEN`] open requests per session; a finished result that
//!   nobody collected is dropped after [`KEEP`].
//!
//! Depends on: vw-agent-tools (`secretspec`, `wire`), vw-keys (project secrets), vw-procmon, vw-gate, tokio
//! (`process`), `secret_grants`, `session`, `killcheck`, `gitcheck`, `safety`. Called by: `tools::call`.
//! Invariants:
//! - No value is logged, put in an error text, a dialog, the grant store or an answer; values live in
//!   `SecretString`s (zeroed on drop) until they are handed to the child's environment.
//! - A request whose dialog text would be too long to show in full is refused before asking (the dialog never
//!   clips, `vw_host::DIALOG_PARAMS_MAX_CHARS`).

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

/// Longest wait of one tool call for the result (under Vibe's 60 s MCP timeout and the binary's 50 s).
pub(crate) const WAIT: Duration = Duration::from_secs(40);
/// Open (waiting or running) requests per session.
pub(crate) const MAX_OPEN: usize = 8;
/// How long an uncollected result is kept.
const KEEP: Duration = Duration::from_secs(15 * 60);
/// The dialog method name.
pub(crate) const METHOD: &str = "vibe-work/secretRun";
/// The third button of the consent dialog.
pub(crate) const ALWAYS_LABEL: &str = "Always for this command";
/// The refusal when this command's grant was revoked while its dialog was open.
const REVOKED_MEANWHILE: &str = "Not run: the user revoked this command's permission while the dialog was open; ask again with secret_run";
/// Longest summary the native dialog shows in full (`vw_host::DIALOG_PARAMS_MAX_CHARS`; a private test keeps
/// the two equal).
pub(crate) const DIALOG_MAX: usize = 1200;

#[derive(Clone, Debug)]
enum RunState {
    WaitingForUser,
    Running,
    Finished(Value),
    Refused(String),
}

struct Run {
    session_id: String,
    state: watch::Receiver<RunState>,
    created: Instant,
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

    /// Drops results older than [`KEEP`], then counts the session's open requests.
    fn open_of(&self, session_id: &str) -> usize {
        let mut runs = self.lock();
        runs.retain(|_, r| r.created.elapsed() < KEEP || !terminal(&r.state.borrow()));
        runs.values()
            .filter(|r| r.session_id == session_id && !terminal(&r.state.borrow()))
            .count()
    }
}

fn terminal(state: &RunState) -> bool {
    matches!(state, RunState::Finished(_) | RunState::Refused(_))
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
    crate::agent_access::check(shared)?;
    if shared.store.view(session_id).is_none() {
        return Err("this session is not open in Vibe Work right now".to_owned());
    }
    match tool {
        names::SECRET_LIST => list(shared, session_id).await,
        names::SECRET_RUN => {
            let id = start(shared, tools, session_id, &args).await?;
            wait(tools, session_id, &id).await
        }
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

/// The project folder of a session: the main working tree of its worktree copy, else its first folder.
pub(crate) fn project_of(shared: &Shared, session_id: &str, folders: &[PathBuf]) -> String {
    let main = shared
        .worktrees
        .records()
        .ok()
        .and_then(|records| records.get(session_id).map(|r| r.repo_root.clone()));
    main.or_else(|| folders.first().cloned())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
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
    let secrets = secrets(shared).await?;
    let project = folders(shared, session_id)
        .map(|f| project_of(shared, session_id, &f))
        .unwrap_or_default();
    let grants = crate::secret_grants::views(shared).await;
    let allowed: Vec<Value> = grants
        .iter()
        .filter(|g| vw_vault::command_grant::same_project(&g.project, &project))
        .map(|g| json!({"command": g.command, "secrets": g.bindings}))
        .collect();
    // The website logins (name, site, user name; never a password) for `browser_act fill`.
    let logins: Vec<Value> = keys(shared)?
        .logins()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.has_password)
        .map(|l| json!({"name": l.name, "site": l.origin, "username": l.username, "id": l.id}))
        .collect();
    Ok(json!({
        "logins": logins,
        "howToSignIn": "browser_act {action: \"fill\", ref: <a password or user-name field from browser_read>, login: \
                        <name>}; the user confirms the password once or always for that site; you never see it",
        "secrets": secrets.iter().map(|s| json!({"id": s.id, "name": s.name, "fields": s.fields, "label": s.label.map(vw_keys::SecretLabel::as_str)})).collect::<Vec<_>>(),
        "alwaysAllowedHere": allowed,
        "howToUse": "secret_run with env [{\"name\": \"VARIABLE\", \"secret\": \"<id or name>\"}]; the user confirms \
                     unless it is always allowed here",
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
                .ok_or_else(|| {
                    let known: Vec<&str> = secrets.iter().map(|s| s.name.as_str()).collect();
                    format!(
                        "no secret named '{}' (secret_list shows: {}); ask the user to add it in Vibe Work \
                         (Settings > Project secrets) or select it in the browser panel and press Save to vault",
                        b.secret.chars().take(60).collect::<String>(),
                        if known.is_empty() { "none".to_owned() } else { known.join(", ") }
                    )
                })?;
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

/// Checks the request, runs the guards, records it and starts the consent-and-run task. Returns the request id.
async fn start(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    args: &Value,
) -> Result<String, String> {
    let spec = secretspec::parse(args)?;
    if tools.secret_runs.open_of(session_id) >= MAX_OPEN {
        return Err(format!(
            "this session already has {MAX_OPEN} secret_run requests waiting or running; collect them with \
             secret_run_status first"
        ));
    }
    let secrets = secrets(shared).await?;
    let bound = bind(&spec, &secrets)?;
    let folders = folders(shared, session_id)?;
    let cwd = inside(&folders, spec.cwd.as_deref())?;
    let project = project_of(shared, session_id, &folders);
    let policy_project = permission_project_of(shared, session_id, &folders)?;
    let env = &tools.config.process_env;
    let program = secretspec::resolve_program(
        &spec.program,
        &cwd,
        path_var(env, "PATH").as_deref(),
        path_var(env, "PATHEXT").as_deref(),
    )
    .ok_or_else(|| {
        format!(
            "program '{}' was not found (not on PATH and not a file in the folder)",
            spec.program.chars().take(80).collect::<String>()
        )
    })?;
    // Pin what the consent binds to first (secret_pin.rs): the program and every argument that names a file are
    // resolved to their canonical paths. That ONE final argv (`pinned.program` + `pinned.args`) is what every check
    // below reads, what the dialog shows, what the grant key covers and what runs (review: no validator differential
    // between the request as written and what is executed).
    let pin_bindings: Vec<(String, String, Option<String>)> = bound
        .iter()
        .map(|b| (b.env.clone(), b.secret.id.clone(), b.field.clone()))
        .collect();
    let (p_program, p_args, p_cwd) = (program.clone(), spec.args.clone(), cwd.clone());
    let pinned = tokio::task::spawn_blocking(move || {
        super::secret_pin::pin(&p_program, &p_args, &p_cwd, &pin_bindings)
    })
    .await
    .map_err(|e| format!("the command could not be checked: {e}"))??;
    let program_text = pinned.program.to_string_lossy().into_owned();
    // The real program may be an interpreter under another name (a link): checked by its real name.
    if let Some(reason) = secretspec::inline_evaluation(&program_text, &pinned.args) {
        return Err(reason);
    }
    let command = command_text(&program_text, &pinned.args);
    // Vibe's card for this exact `secret_run` call (program, arguments, which secret goes where) came
    // before this call; the user's yes there answers the git and kill guards' "may this command run" question only.
    // It never stands in for the consent dialog below (giving a secret is another question, native dialog only).
    let card = crate::one_ask::of(shared).is_some_and(|ledger| {
        ledger.card_said_yes(
            session_id,
            &format!(
                "mcp_{}.{}",
                vw_agent_tools::wire::SERVER_NAME,
                names::SECRET_RUN
            ),
            args,
        )
    });
    let asked = crate::one_ask::Asked::Card(card);
    super::session::check_config_command(shared, &cwd, &command)?;
    let found = vw_agent_tools::killparse::scan(&command);
    super::killcheck::guard(shared, session_id, &command, &found, asked).await??;
    super::gitcheck::guard(shared, session_id, &cwd, &command, asked).await?;
    crate::safety::guard(shared, &command, Some(&cwd))?;
    let dialog = dialog_request(shared, session_id, &spec, &bound, &cwd, &project, &pinned);
    if dialog.summary.chars().count() > DIALOG_MAX {
        return Err(format!(
            "this request is too long to show the user in full (over {DIALOG_MAX} characters); put the \
             arguments into a script file and run that"
        ));
    }
    let id = vw_agent_tools::credential::new_credential()
        .map(|c| format!("secret-run-{}", &c[..16]))
        .map_err(|e| format!("no request id could be made: {e}"))?;
    let (sender, receiver) = watch::channel(RunState::WaitingForUser);
    tools.secret_runs.lock().insert(
        id.clone(),
        Run {
            session_id: session_id.to_owned(),
            state: receiver,
            created: Instant::now(),
        },
    );
    let job = Job {
        shared: shared.clone(),
        tools: tools.clone(),
        session_id: session_id.to_owned(),
        spec,
        bound,
        program: pinned.program.clone(),
        pinned,
        cwd,
        project,
        policy_project,
        dialog,
    };
    tokio::spawn(async move {
        let outcome = job.consent_and_run(&sender).await;
        let _ = sender.send(match outcome {
            Ok(result) => RunState::Finished(result),
            Err(reason) => RunState::Refused(reason),
        });
    });
    Ok(id)
}

fn dialog_params(
    program: &str,
    spec: &RunSpec,
    bound: &[Bound],
    cwd: &Path,
    project: &str,
    pinned: &super::secret_pin::Pinned,
) -> Value {
    json!({
        "runs exactly (no shell)": command_text(program, &pinned.args),
        "files checked before it starts": pinned.files.iter().skip(1).map(|(p, _)| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "program": program,
        "args": pinned.args,
        "folder": cwd.to_string_lossy(),
        "secrets": bound.iter().map(|b| json!({"variable": b.env, "secret": b.secret.name})).collect::<Vec<_>>(),
        "purpose (the agent's words)": spec.purpose,
        "project": project,
        "stopsAfterSeconds": spec.timeout_s,
    })
}

fn dialog_request(
    shared: &Shared,
    session_id: &str,
    spec: &RunSpec,
    bound: &[Bound],
    cwd: &Path,
    project: &str,
    pinned: &super::secret_pin::Pinned,
) -> ConfirmationRequest {
    let title = shared
        .names
        .title_of(&shared.store, session_id)
        .unwrap_or_else(|| session_id.to_owned());
    let names: Vec<String> = bound
        .iter()
        .map(|b| {
            format!(
                "{} into {}",
                vw_gate::dialog_quote(&b.secret.name),
                vw_gate::dialog_quote(&b.env),
            )
        })
        .collect();
    let summary = super::secret_dialog::summary(&super::secret_dialog::Dialog {
        title: &title,
        project,
        cwd,
        purpose: &spec.purpose,
        timeout_s: spec.timeout_s,
        names: &names,
        pinned,
    });
    let params = dialog_params(
        &pinned.program.to_string_lossy(),
        spec,
        bound,
        cwd,
        project,
        pinned,
    );
    ConfirmationRequest {
        method: METHOD.to_owned(),
        requester: Requester::Agent,
        summary,
        params,
        always: Some(ALWAYS_LABEL.to_owned()),
    }
}

/// Waits up to [`WAIT`] for request `id` of `session_id`.
async fn wait(tools: &AgentTools, session_id: &str, id: &str) -> Result<Value, String> {
    let receiver = {
        let runs = tools.secret_runs.lock();
        match runs.get(id) {
            Some(run) if run.session_id == session_id => run.state.clone(),
            _ => {
                return Err(
                    "no such secret_run request in this session (results are kept 15 minutes)"
                        .to_owned(),
                );
            }
        }
    };
    let mut receiver = receiver;
    let _ = tokio::time::timeout(WAIT, receiver.wait_for(terminal)).await;
    let state = receiver.borrow().clone();
    match state {
        RunState::Finished(result) => {
            tools.secret_runs.lock().remove(id);
            Ok(result)
        }
        RunState::Refused(reason) => {
            tools.secret_runs.lock().remove(id);
            Err(reason)
        }
        RunState::WaitingForUser => Ok(json!({
            "status": "waiting for the user's answer",
            "requestId": id,
            "next": "call secret_run_status with this requestId",
        })),
        RunState::Running => Ok(json!({
            "status": "running",
            "requestId": id,
            "next": "call secret_run_status with this requestId",
        })),
    }
}

/// How the request was approved.
#[derive(Clone)]
enum Approval {
    /// Every named entry has a standing grant for this project.
    EntryAllowed,
    /// "Allow once"; the time the dialog opened (a revoke of this command after it refuses the run).
    Once(i64),
    /// A new "always" grant (its id).
    Always(String),
    /// An existing "always" grant (its id).
    Grant(String),
}

impl Approval {
    fn as_str(&self) -> &'static str {
        match self {
            Self::EntryAllowed => "entryAllowed",
            Self::Once(_) => "allowedOnce",
            Self::Always(_) => "allowedAlways",
            Self::Grant(_) => "grant",
        }
    }
}

struct Job {
    shared: Arc<Shared>,
    tools: Arc<AgentTools>,
    session_id: String,
    spec: RunSpec,
    bound: Vec<Bound>,
    program: PathBuf,
    pinned: super::secret_pin::Pinned,
    cwd: PathBuf,
    project: String,
    policy_project: crate::entry_permissions::Identity,
    dialog: ConfirmationRequest,
}

impl Job {
    fn log(&self, decision: &str) {
        let session = self
            .shared
            .names
            .title_of(&self.shared.store, &self.session_id)
            .unwrap_or_else(|| self.session_id.clone());
        self.shared.secret_grants.log(SecretUseView {
            at_ms: now_ms(),
            session,
            project: self.project.clone(),
            program: self.program.to_string_lossy().into_owned(),
            bindings: self.bindings_text(),
            decision: decision.to_owned(),
        });
        self.shared.emit(crate::event::CoreEvent::SecretsChanged {});
    }

    fn bindings_text(&self) -> Vec<String> {
        self.bound
            .iter()
            .map(|b| format!("{} <- {}", b.env, b.secret.name))
            .collect()
    }

    /// The approval, or the use log's decision word (`declined`: the user's answer; `refused`: the answer could not
    /// be honoured) and the reason.
    async fn consent(&self, all_allowed: bool) -> Result<Approval, (&'static str, String)> {
        if all_allowed {
            return Ok(Approval::EntryAllowed);
        }
        // Only an "always" grant of exactly this pinned command in this project counts.
        // read from the vault now; a vault that cannot be read means no grant, and the user is asked.
        if let Some(id) =
            crate::secret_grants::find(&self.shared, &self.pinned.fingerprint, &self.project).await
        {
            return Ok(Approval::Grant(id));
        }
        // A revoke of this command made while the dialog is open (its `revokedMs` at or after
        // this moment) wins over the answer.
        let asked = now_ms();
        match self.shared.confirmer.confirm(&self.dialog).await {
            Confirmation::Approved => Ok(Approval::Once(asked)),
            Confirmation::ApprovedAlways => {
                let new = crate::secret_grants::NewGrant {
                    project: self.project.clone(),
                    fingerprint: self.pinned.fingerprint.clone(),
                    command: command_text(&self.program.to_string_lossy(), &self.pinned.args),
                    bindings: self.bindings_text(),
                    secret_ids: self.bound.iter().map(|b| b.secret.id.clone()).collect(),
                };
                match crate::secret_grants::grant_always(&self.shared, new, asked, now_ms()).await {
                    Ok(id) => Ok(Approval::Always(id)),
                    Err(crate::secret_grants::NotGranted::RevokedMeanwhile) => {
                        Err(("refused", REVOKED_MEANWHILE.to_owned()))
                    }
                    Err(crate::secret_grants::NotGranted::Vault(reason)) => Err(("refused", format!(
                        "Not run: the user chose \"{ALWAYS_LABEL}\", but the permission could not be stored in their \
                         vault ({reason}). Ask the user to unlock the vault, then try again."
                    ))),
                }
            }
            Confirmation::Declined => Err((
                "declined",
                "Not run: the user did not allow this use of their secret. Continue without it, or ask the user."
                    .to_owned(),
            )),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "consent, pin and final policy checks are one ordered flow"
    )]
    async fn consent_and_run(self, state: &watch::Sender<RunState>) -> Result<Value, String> {
        let keys = keys(&self.shared)?;
        let mut policies = Vec::with_capacity(self.bound.len());
        for binding in &self.bound {
            let policy = self
                .policy_project
                .policy(keys, &binding.secret.id)
                .await
                .map_err(|_| {
                    "Not run: this entry's project permission could not be checked".to_owned()
                })?;
            if policy.permission == vw_keys::EntryPermission::Never {
                self.log("refused");
                return Err(format!(
                    "Not run: this entry is set to {} in this project",
                    vw_vocabulary::label("vaultPermissions", "never")
                ));
            }
            policies.push((binding.secret.id.clone(), policy));
        }
        let all_allowed = policies
            .iter()
            .all(|(_, policy)| policy.permission == vw_keys::EntryPermission::Allowed);
        let approval = match self.consent(all_allowed).await {
            Ok(approval) => approval,
            Err((decision, reason)) => {
                self.log(decision);
                return Err(reason);
            }
        };
        self.log(approval.as_str());
        let _ = state.send(RunState::Running);
        crate::agent_access::check(&self.shared)?;
        let mut values: Vec<(String, SecretString)> = Vec::new();
        for b in &self.bound {
            let value = keys
                .secret_value(b.secret.id.clone(), b.field.clone())
                .await
                .map_err(|e| {
                    format!(
                        "Not run: the secret '{}' could not be read ({e})",
                        b.secret.name
                    )
                })?;
            values.push((b.env.clone(), value));
        }
        // Run exactly what was allowed: every pinned file still has
        // the content the dialog covered, and a grant was not revoked meanwhile; the grant's use counts here.
        // the check keeps the files open until the run ends (`secret_pin::hold`); Linux starts the program
        // through the very descriptor that was hashed. the grant check after the hold reads the vault
        // (an await), which cannot change the held files; nothing else is awaited before the start.
        let pinned = self.pinned.clone();
        let held = tokio::task::spawn_blocking(move || super::secret_pin::hold(&pinned))
            .await
            .map_err(|e| format!("Not run: the command could not be checked ({e})"))??;
        match &approval {
            Approval::EntryAllowed => {}
            Approval::Grant(id) | Approval::Always(id) => {
                // The vault's current record, read again (a revoke in between wins; a read error refuses).
                if !crate::secret_grants::holds(
                    &self.shared,
                    &self.pinned.fingerprint,
                    &self.project,
                )
                .await
                {
                    self.log("refused");
                    return Err(
                        "Not run: the user revoked this permission meanwhile; ask again with secret_run".to_owned(),
                    );
                }
                self.shared.secret_grants.used(id);
            }
            Approval::Once(asked) => {
                if crate::secret_grants::revoked_during(
                    &self.shared,
                    &self.pinned.fingerprint,
                    &self.project,
                    *asked,
                )
                .await
                {
                    self.log("refused");
                    return Err(REVOKED_MEANWHILE.to_owned());
                }
            }
        }
        if let Some(hook) = &self.tools.config.secret_run_before_start {
            (hook.0)(&self.pinned.program);
        }
        for (id, policy) in &policies {
            match self.policy_project.policy(keys, id).await {
                Ok(current) if current == *policy => {}
                _ => {
                    self.log("refused");
                    return Err(
                        "Not run: an entry permission changed meanwhile; ask the user to try again"
                            .to_owned(),
                    );
                }
            }
        }
        crate::agent_access::check(&self.shared)?;
        let mut result = super::secret_exec::execute(
            &self.shared,
            &super::secret_exec::Exec {
                process_env: &self.tools.config.process_env,
                session_id: &self.session_id,
                program: &self.pinned.program,
                held: &held,
                args: &self.pinned.args,
                cwd: &self.cwd,
                timeout: Duration::from_secs(self.spec.timeout_s),
            },
            &values,
        )
        .await?;
        result["approval"] = json!(approval.as_str());
        result["secretsUsed"] = json!(
            self.bound
                .iter()
                .map(|b| json!({"variable": b.env, "secret": b.secret.name}))
                .collect::<Vec<_>>()
        );
        Ok(result)
    }
}
