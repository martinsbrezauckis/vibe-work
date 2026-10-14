//! Vault fill: `browser_act {action: "fill", ref, login}` puts a vault login into a sign-in field of the session's
//! built-in browser. The agent names the field (a ref of its latest read) and the login (its name, as `secret_list`
//! shows it); it may see the user name, never the password. Adapted from ShellX's vault fill
//! design, the order is strict: every check first, then the grant, then the password, then the write.
//!
//! Order: (1) vw-browser `fill_check` (Stop, the Lan rule, a current host-held observation listing the ref as a
//! password or user-name field, a secure page, a form that submits only to the page's origin, then the same checks in
//! the page): no vault access at all; (2) the login lookup (metadata) and "this login is for this site"; (3) a user
//! name goes in without a grant (it is not a secret); a password needs an active "always for this site" grant (read
//! now) or the native dialog (Allow once / Always for this site / Don't allow); (4) an "always" grant is read again
//! right before the password; (5) `KeyService::login_password` (counted, registered as a stored secret); (6) vw-browser
//! `fill_write` (the page script repeats every check in the same call that writes).
//!
//! Who decides: only the native dialog. Its method is in `vault_scope::DIALOG_METHODS` (the confirmer reads it), so a Debug API
//! request (on the vault the Debug API has the agent's rights) gets the same dialog and never an automatic
//! answer; no client can send an "always".
//!
//! Depends on: vw-browser (`fill_check`, `fill_write`), vw-keys (logins), vw-vault (`login::fits`), vw-gate (dialog),
//! `fill_grants`, `secret_grants` (the use log). Called by: `agent_tools/browser_tools` (the agent), `browser` (the
//! `browser` command's `act` for the window and the Debug API).
//! Invariants: the password is never in an answer, an error, a log line, the action log or the use log; a refused
//! check never reaches the vault (`KeyService::login_counts` proves it in the tests).

use serde_json::{Value, json};
use vw_browser::{Actor, Browser, FillRefusal, FillTarget, LoginRole};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_keys::Login;

use std::sync::Arc;

use vw_preview::PortOwner;

use crate::core::Shared;
use crate::fill_grants;
use crate::preview_ports::{port_owner, session_ports};
use crate::secrets_view::SecretUseView;

/// The dialog's method name (native dialog only: the confirmer never answers it for the Debug API).
pub(crate) const METHOD: &str = "vibe-work/fillLogin";
/// The dialog's "always" button.
pub(crate) const ALWAYS_LABEL: &str = "Always for this site";

/// One fill as the agent (or the Debug API) asked for it.
struct Wanted {
    reference: String,
    login: String,
}

fn parse(args: &Value) -> Result<Wanted, String> {
    let object = args.as_object().ok_or("fill takes {ref, login}")?;
    if let Some(other) = object
        .keys()
        .find(|k| !matches!(k.as_str(), "action" | "ref" | "login" | "target"))
    {
        return Err(format!(
            "fill takes only ref and login (not '{}')",
            other.chars().take(20).collect::<String>()
        ));
    }
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    Ok(Wanted {
        reference: text("ref").ok_or("fill needs ref: the field's ref from browser_read")?,
        login: text("login")
            .ok_or("fill needs login: the login's name as secret_list shows it under logins")?,
    })
}

fn refused(check: &str, text: &str) -> String {
    format!("not filled ({check}): {text}")
}

/// The login named `wanted` (name, id, site or host) among those that fit the page's host.
fn pick<'a>(logins: &'a [Login], wanted: &str, target: &FillTarget) -> Result<&'a Login, String> {
    let w = wanted.to_lowercase();
    let named: Vec<&Login> = logins
        .iter()
        .filter(|l| {
            l.name.to_lowercase() == w
                || l.id == wanted
                || l.origin.eq_ignore_ascii_case(wanted)
                || l.host == w
        })
        .collect();
    let fitting: Vec<&Login> = named
        .iter()
        .copied()
        .filter(|l| vw_vault::login::fits(l, &target.origin))
        .collect();
    match (named.len(), fitting.as_slice()) {
        (_, [one]) => Ok(one),
        (0, _) => Err(refused(
            "login",
            &format!(
                "no login named '{}' in the user's vault (secret_list shows the logins): ask the user to add it in \
                 Settings > Project secrets",
                wanted.chars().take(60).collect::<String>()
            ),
        )),
        (_, []) => Err(refused(
            "login",
            &format!(
                "the login '{}' is for {}, not for this page ({}): a login is filled only on its own site",
                named[0].name, named[0].origin, target.origin
            ),
        )),
        (_, _) => Err(refused(
            "login",
            "more than one login with that name fits this site: use the login's id from secret_list",
        )),
    }
}

/// The dialog: the session, the site, the login's name and user name, the field; never the password. `local` (a page
/// on this computer): who listens on its port, and no "always" button (any local program, the
/// agent's own servers included, can listen on a loopback port, so the user decides every time).
fn consent(
    title: &str,
    target: &FillTarget,
    login: &Login,
    requester: Requester,
    local: Option<&str>,
) -> ConfirmationRequest {
    // "Always" only for the agent on a web site: never on this computer, never for a Debug API request (the
    // Debug API has the agent's rights on the vault and never creates a permission, even through the user's button).
    let offer_always = local.is_none() && requester != Requester::DebugApi;
    ConfirmationRequest {
        method: METHOD.to_owned(),
        requester,
        summary: format!(
            "The agent of the session {} wants to sign in on {} with your saved login {}{}: Vibe Work puts the \
             password into the field {} on that page. The agent never sees the password.{}",
            vw_gate::dialog_quote(title),
            vw_gate::dialog_quote(&target.origin),
            vw_gate::dialog_quote(&login.name),
            if login.username.is_empty() {
                String::new()
            } else {
                format!(" (user {})", vw_gate::dialog_quote(&login.username))
            },
            vw_gate::dialog_quote(&target.label),
            local.map_or_else(String::new, |holder| format!(
                " This page is on this computer; its port is held by {}. Allow it only if that is the app you are \
                 testing (you are asked every time here).",
                vw_gate::dialog_quote(holder)
            )),
        ),
        params: json!({
            "site": target.origin,
            "login": login.name,
            "username": login.username,
            "field": target.label,
            "portHeldBy": local,
        }),
        always: offer_always.then(|| ALWAYS_LABEL.to_owned()),
    }
}

/// Runs one fill (module docs) and answers `{ok, Outcome, field, login, site}`.
///
/// # Errors
/// A plain-text refusal naming the failed check (`not filled (<check>): ...`), never the password.
#[allow(
    clippy::too_many_lines,
    reason = "fill validation and final permission checks are one ordered flow"
)]
pub(crate) async fn fill(
    shared: &Arc<Shared>,
    driver: &Browser,
    session_id: &str,
    args: &Value,
    by: Actor,
    requester: Requester,
) -> Result<Value, String> {
    crate::agent_access::check(shared)?;
    let owner = crate::previews::hub(shared)
        .map_err(|error| error.message)?
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| "browser page has no session owner".to_owned())?;
    let wanted = parse(args)?;
    // (1) every check before the vault is touched.
    let target = driver
        .fill_check(session_id, &wanted.reference, by)
        .await
        .map_err(|e| note(driver, session_id, by, &wanted.reference, &e))?;
    let fail = |check: &'static str, text: String| {
        let refusal = FillRefusal { check, text };
        note(driver, session_id, by, &wanted.reference, &refusal)
    };
    // (2) the login.
    let keys = shared.config.keys.as_ref().ok_or_else(|| {
        fail(
            "vault",
            "Vibe Work has no vault connected, so no login can be filled".to_owned(),
        )
    })?;
    let logins = keys.logins().await.map_err(|e| {
        fail(
            "vault",
            if vw_keys::is_locked(&e) {
                "the user's vault is locked: ask the user to unlock it in Vibe Work (Settings > Account), then try \
                 again"
                    .to_owned()
            } else {
                format!("the vault could not be read ({e})")
            },
        )
    })?;
    let login = pick(&logins, &wanted.login, &target).inspect_err(|text| {
        driver.fill_refused(session_id, by, &wanted.reference, text);
    })?;
    let folders =
        crate::agent_tools::session::folders(shared, &owner).map_err(|e| fail("project", e))?;
    let project = crate::agent_tools::permission_project_of(shared, &owner, &folders)
        .map_err(|e| fail("project", e))?;
    let policy = project.policy(keys, &login.id).await.map_err(|_| {
        fail(
            "permission",
            "the entry permission could not be checked".to_owned(),
        )
    })?;
    if policy.permission == vw_keys::EntryPermission::Never {
        return Err(fail(
            "permission",
            format!(
                "this entry is set to {} in this project",
                vw_vocabulary::label("vaultPermissions", "never")
            ),
        ));
    }
    if target.role == LoginRole::Username {
        if login.username.is_empty() {
            return Err(fail("login", "this login has no user name".to_owned()));
        }
        if project.policy(keys, &login.id).await.ok().as_ref() != Some(&policy) {
            return Err(fail(
                "permission",
                "the entry permission changed meanwhile".to_owned(),
            ));
        }
        driver
            .fill_write(session_id, &target, &login.username, by)
            .await
            .map_err(|e| refused(e.check, &e.text))?;
        return Ok(answer(&target, login));
    }
    if !login.has_password {
        return Err(fail("login", "this login has no password".to_owned()));
    }
    // (3) + (4) the grant, or the user's answer.
    let decision = decide(
        shared,
        &owner,
        &target,
        login,
        requester,
        policy.permission == vw_keys::EntryPermission::Allowed,
    )
    .await
    .map_err(|(decision, reason)| {
        log(shared, &owner, &target, login, decision);
        fail("grant", reason)
    })?;
    crate::agent_access::check(shared)?;
    // (5) the password, only now.
    let password = keys
        .login_password(login.id.clone())
        .await
        .map_err(|e| fail("vault", format!("the password could not be read ({e})")))?;
    crate::agent_access::check(shared)?;
    if project.policy(keys, &login.id).await.ok().as_ref() != Some(&policy) {
        return Err(fail(
            "permission",
            "the entry permission changed meanwhile".to_owned(),
        ));
    }
    // (6) the write (the page repeats every check).
    let written = driver
        .fill_write(session_id, &target, password.expose(), by)
        .await;
    drop(password);
    log(
        shared,
        &owner,
        &target,
        login,
        if written.is_ok() { decision } else { "refused" },
    );
    written.map_err(|e| refused(e.check, &e.text))?;
    tracing::info!(decision, "a vault login was filled into a page");
    Ok(answer(&target, login))
}

/// The user's decision for a password fill: an active "always" grant (read again right before the password, so a revoke
/// in between wins), else the native dialog. `Err((log decision, reason))` when it may not happen.
async fn decide(
    shared: &Arc<Shared>,
    session_id: &str,
    target: &FillTarget,
    login: &Login,
    requester: Requester,
    allowed: bool,
) -> Result<&'static str, (&'static str, String)> {
    // A local listener can change hands while Vibe Work stays open; always asks for that port.
    if allowed && !vw_vault::login::is_loopback_host(&target.host) {
        return Ok("entryAllowed");
    }
    // A page on this computer never uses (or gets) an "always" grant: the dialog asks every time and names the port's
    // holder.
    let local = if vw_vault::login::is_loopback_host(&target.host) {
        Some(local_holder(shared, session_id, &target.origin).await)
    } else {
        None
    };
    if local.is_none()
        && let Some(id) = fill_grants::find(shared, &login.id, &target.origin).await
    {
        if !fill_grants::holds(shared, &login.id, &target.origin).await {
            return Err((
                "refused",
                "the user revoked this permission meanwhile: the next fill asks them".to_owned(),
            ));
        }
        shared.secret_grants.fill_uses.used(&id);
        return Ok("grant");
    }
    let title = shared
        .names
        .title_of(&shared.store, session_id)
        .unwrap_or_else(|| session_id.to_owned());
    let asked = fill_grants::now_ms();
    match shared
        .confirmer
        .confirm(&consent(&title, target, login, requester, local.as_deref()))
        .await
    {
        Confirmation::Declined => Err((
            "declined",
            "the user did not allow it. Continue without signing in, or ask the user".to_owned(),
        )),
        Confirmation::ApprovedAlways if local.is_none() && requester != Requester::DebugApi => {
            let id =
                fill_grants::grant_always(shared, &login.id, &login.name, &target.origin, asked)
                    .await
                    .map_err(|reason| ("refused", reason))?;
            shared.secret_grants.fill_uses.used(&id);
            Ok("allowedAlways")
        }
        Confirmation::Approved | Confirmation::ApprovedAlways => {
            if fill_grants::revoked_during(shared, &login.id, &target.origin, asked).await {
                return Err((
                    "refused",
                    "the permission was revoked while the dialog was open; ask again".to_owned(),
                ));
            }
            Ok("allowedOnce")
        }
    }
}

/// Who listens on the port of a page on this computer, from the process monitor's facts (the same owners the
/// preview's refusals name).
async fn local_holder(shared: &Arc<Shared>, session_id: &str, origin: &str) -> String {
    let Some(port) = url::Url::parse(origin)
        .ok()
        .and_then(|u| u.port_or_known_default())
    else {
        return "an unknown program".to_owned();
    };
    let (owned, id) = (shared.clone(), session_id.to_owned());
    let found = tokio::task::spawn_blocking(move || {
        let ports = session_ports(&owned, &id).ok()?;
        let holder = port_owner(&owned, &ports, &id, port, true);
        let process = ports
            .targets
            .iter()
            .find(|t| t.port == port)
            .and_then(|t| t.process_name.clone().map(|n| (n, t.pid)));
        Some((holder, process))
    })
    .await
    .ok()
    .flatten();
    match found {
        Some((PortOwner::ThisSession, Some((name, pid)))) => format!(
            "{name}{} started in this session (port {port})",
            pid.map_or_else(String::new, |p| format!(" (pid {p})"))
        ),
        Some((PortOwner::ThisSession, None)) => format!("this session (port {port})"),
        Some((PortOwner::OtherSession(name) | PortOwner::OtherApp(name), _)) => {
            format!("{name} (port {port})")
        }
        Some((PortOwner::NotProven, _)) => {
            format!("a program in this session's folder that Vibe Work did not start (port {port})")
        }
        Some((PortOwner::Nobody, _)) => format!("no program right now (port {port})"),
        None => format!("an unknown program (port {port})"),
    }
}

fn answer(target: &FillTarget, login: &Login) -> Value {
    json!({
        "ok": true,
        "Outcome": format!(
            "filled the {} field with the login '{}' from the user's vault (you never see the password); submit \
             the form with a normal click",
            target.role.as_str(),
            login.name
        ),
        "field": target.role.as_str(),
        "login": login.name,
        "site": target.origin,
    })
}

fn note(driver: &Browser, session_id: &str, by: Actor, reference: &str, e: &FillRefusal) -> String {
    driver.fill_refused(session_id, by, reference, &e.text);
    refused(e.check, &e.text)
}

/// One line in the use log Settings shows (no value).
fn log(shared: &Shared, session_id: &str, target: &FillTarget, login: &Login, decision: &str) {
    let session = shared
        .names
        .title_of(&shared.store, session_id)
        .unwrap_or_else(|| session_id.to_owned());
    shared.secret_grants.log(SecretUseView {
        at_ms: fill_grants::now_ms(),
        session,
        project: String::new(),
        program: format!("browser fill on {}", target.origin),
        bindings: vec![format!("password <- {}", login.name)],
        decision: decision.to_owned(),
    });
}
