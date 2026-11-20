//! Voice permission answers: one owned session, fresh challenge and bound audio stream. Called by speech.
//! Decisions still use the card adapter's atomic claim; unknown tools never gain approval by voice.
use crate::{
    CommandResult, CoreError, ErrorCode, Origin, SpeechRequest, SpeechResult, core::Shared,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::time::{Duration, Instant};
use vw_ask::{AnsweredBy, Ask, Choice, Surface};
use vw_platform::sync::LockRecover;

#[derive(Default)]
pub(crate) struct Prompts(Mutex<State>);
#[derive(Default)]
struct State {
    owner: Option<String>,
    previous: u8,
    current: Option<Challenge>,
}
#[derive(Clone)]
struct Challenge {
    ticket: String,
    ask: Ask,
    detail: Value,
    number: u8,
    allow: bool,
    until: Instant,
    first_started: Option<Instant>,
    starts: u8,
    misses: u8,
    stream: Option<String>,
    quiet: Option<Arc<vw_preview::AudioQuiet>>,
    _lease: Option<Arc<crate::session_lease::Lease>>,
}
impl Prompts {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock_or_recover()
    }
}
fn refused() -> CoreError {
    CoreError::new(ErrorCode::CallbackRefused, "Answer on the card.")
}
fn ended() -> CoreError {
    CoreError::new(
        ErrorCode::CallbackRefused,
        "Voice answers for this question ended. Answer on the card in the chat.",
    )
}
fn arm_window(c: &mut Challenge, now: Instant) -> Result<Duration, CoreError> {
    let first = *c.first_started.get_or_insert(now);
    let cap = first + Duration::from_secs(180);
    if c.starts >= 3 || now >= cap || (c.starts > 0 && now >= c.until) {
        return Err(ended());
    }
    c.starts += 1;
    c.until = std::cmp::min(now + Duration::from_secs(60), cap);
    Ok(c.until - now)
}
fn defer_for_reprompt(c: &mut Challenge, now: Instant) -> bool {
    let Some(first) = c.first_started else {
        return false;
    };
    if c.starts >= 3 || now >= first + Duration::from_secs(180) {
        return false;
    }
    c.until = std::cmp::min(
        now + Duration::from_secs(60),
        first + Duration::from_secs(180),
    );
    true
}
fn watch_deadline(shared: &Arc<Shared>, ticket: String, deadline: Instant) {
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        tokio::time::sleep_until(deadline).await;
        if let Some(shared) = weak.upgrade() {
            let mut state = shared.voice_prompts.lock();
            let expired = state
                .current
                .as_ref()
                .is_some_and(|c| c.ticket == ticket && Instant::now() >= c.until);
            let stream = if expired {
                state.current.take().and_then(|c| c.stream)
            } else {
                None
            };
            drop(state);
            if let Some(stream) = stream {
                shared.dictations.cancel(&stream);
            }
        }
    });
}
fn result(result: SpeechResult) -> CommandResult {
    CommandResult::Speech { result }
}
fn status(status: &str) -> CommandResult {
    result(SpeechResult::PromptAnswer {
        status: status.into(),
        heard: None,
    })
}
/// A miss: `status` with the words heard (cut to 200 characters), also written to the local diagnostics log the same
/// way the window logs a question card's miss (`SpeechRequest::Heard`). Nothing is sent anywhere.
fn missed(status: &str, text: &str) -> CommandResult {
    let heard: String = text.trim().chars().take(200).collect();
    tracing::info!(heard = %heard, "voice answer not understood");
    result(SpeechResult::PromptAnswer {
        status: status.into(),
        heard: Some(heard),
    })
}
fn clear(shared: &Shared, ticket: Option<&str>) {
    let mut state = shared.voice_prompts.lock();
    let matches = ticket.is_none()
        || state
            .current
            .as_ref()
            .is_some_and(|c| Some(c.ticket.as_str()) == ticket);
    if matches
        && let Some(c) = state.current.take()
        && let Some(stream) = c.stream
    {
        shared.dictations.cancel(&stream);
    }
}
pub(crate) fn cancel_stream(shared: &Shared, stream: &str) {
    let ticket = shared
        .voice_prompts
        .lock()
        .current
        .as_ref()
        .filter(|c| c.stream.as_deref() == Some(stream))
        .map(|c| c.ticket.clone());
    if let Some(ticket) = ticket {
        clear(shared, Some(&ticket));
    }
}

fn available(shared: &Shared) -> bool {
    !shared
        .asks
        .list()
        .iter()
        .any(|a| a.surface == Surface::Native || a.person_only)
}

/// Approval is restricted to typed local file operations. Shells and unknown tools can only be refused;
/// native, outbound, credentials and browser actions never enter this voice path.
fn policy(ask: &Ask, detail: &Value) -> Option<bool> {
    if ask.surface != Surface::Card
        || ask.person_only
        || ask.kind != "approval"
        || ask.always_setting.is_some()
        || !ask.offers.contains(&Choice::No)
    {
        return None;
    }
    let tool = detail["effect"]["toolName"].as_str()?;
    if detail["kind"] != "approval" {
        return None;
    }
    if tool.starts_with("mcp_")
        || tool.contains("browser")
        || tool.contains("secret")
        || tool.contains("network")
        || tool.contains("http")
    {
        return None;
    }
    let local = matches!(
        tool,
        "file_system.read_file"
            | "file_system.write_file"
            | "file_system.replace_in_file"
            | "file_system.search"
            | "file_system.grep"
            | "file_system.glob"
    );
    let permissions = detail.get("requiredPermissions");
    if permissions.is_some_and(|p| !p.is_array()) {
        return None;
    }
    let local_scope = permissions
        .and_then(Value::as_array)
        .is_none_or(|ps| ps.iter().all(|p| p["scope"] == "file_pattern"));
    if permissions
        .and_then(Value::as_array)
        .is_some_and(|ps| ps.iter().any(|p| p["scope"] == "url_pattern"))
    {
        return None;
    }
    let within_level = detail.get("reason").is_none_or(Value::is_null)
        && detail.get("vwReason").is_none_or(Value::is_null);
    if !within_level {
        return None;
    }
    Some(local && local_scope && ask.offers.contains(&Choice::Once))
}
fn card(shared: &Shared, session: &str, callback: &str) -> Result<(Ask, Value), CoreError> {
    let ask = shared
        .asks
        .get(&format!("callback:{callback}"))
        .ok_or_else(refused)?;
    if ask.session.as_deref() != Some(session) || !available(shared) {
        return Err(refused());
    }
    let snapshot = crate::sessions::snapshot(shared, session)?;
    if snapshot.state["session"]["archivedAt"].is_number() {
        return Err(refused());
    }
    let entry = snapshot.state["history"]
        .as_array()
        .and_then(|h| {
            h.iter()
                .find(|e| e["callbackId"] == callback || e["id"] == callback)
        })
        .ok_or_else(refused)?;
    if entry["vwSourceSession"].is_string() {
        return Err(refused());
    }
    Ok((ask, entry["detail"].clone()))
}
fn checked(shared: &Shared, ticket: &str) -> Result<Challenge, CoreError> {
    let state = shared.voice_prompts.lock();
    let c = state
        .current
        .as_ref()
        .filter(|c| c.ticket == ticket && state.owner == c.ask.session && Instant::now() < c.until)
        .cloned()
        .ok_or_else(refused)?;
    drop(state);
    let callback = c.ask.id.strip_prefix("callback:").ok_or_else(refused)?;
    let (ask, detail) = card(
        shared,
        c.ask.session.as_deref().ok_or_else(refused)?,
        callback,
    )?;
    if ask != c.ask || detail != c.detail || policy(&ask, &detail) != Some(c.allow) {
        return Err(refused());
    }
    Ok(c)
}

/// Every audio append rechecks the prompt lifetime and the origin; ordinary dictation retains its existing policy.
pub(crate) fn audio_allowed(
    shared: &Shared,
    origin: Origin,
    stream: &str,
) -> Result<(), CoreError> {
    let ticket = shared
        .voice_prompts
        .lock()
        .current
        .as_ref()
        .filter(|c| c.stream.as_deref() == Some(stream))
        .map(|c| c.ticket.clone());
    if let Some(ticket) = ticket
        && (crate::command_origin::debug_api_acts(shared, origin)
            || checked(shared, &ticket).is_err())
    {
        clear(shared, Some(&ticket));
        return Err(refused());
    }
    Ok(())
}

// Kept separate so the full byte distribution is checked without relying on sampled randomness.
fn number_candidate(byte: u8, previous: u8) -> Option<u8> {
    if byte >= 180 {
        return None;
    }
    let number = 10 + byte % 90;
    (number != previous).then_some(number)
}

fn prepare(
    shared: &Arc<Shared>,
    session: &str,
    callback: &str,
) -> Result<CommandResult, CoreError> {
    if shared.voice_prompts.lock().owner.as_deref() != Some(session) {
        return Err(refused());
    }
    let (ask, detail) = card(shared, session, callback)?;
    let Some(allow) = policy(&ask, &detail) else {
        return Ok(result(SpeechResult::Prompt {
            ticket: None,
            number: None,
        }));
    };
    // Only the oldest card can be armed; new arrivals cannot silently replace what the person heard.
    if shared.asks.list().iter().any(|a| {
        a.session.as_deref() == Some(session)
            && a.surface == Surface::Card
            && (a.opened_at, &a.id) < (ask.opened_at, &ask.id)
    }) {
        return Err(refused());
    }
    let mut random = [0_u8; 17];
    getrandom::fill(&mut random).map_err(|_| refused())?;
    let ticket = hex::encode(&random[..16]);
    let mut state = shared.voice_prompts.lock();
    if state.current.is_some() {
        return Err(refused());
    }
    let number = loop {
        if let Some(number) = number_candidate(random[16], state.previous) {
            break number;
        }
        getrandom::fill(&mut random[16..]).map_err(|_| refused())?;
    };
    state.previous = number;
    state.current = Some(Challenge {
        ticket: ticket.clone(),
        ask,
        detail,
        number,
        allow,
        until: Instant::now() + Duration::from_secs(120),
        first_started: None,
        starts: 0,
        misses: 0,
        stream: None,
        quiet: None,
        _lease: shared.leases.claim(session)?,
    });
    drop(state);
    watch_deadline(
        shared,
        ticket.clone(),
        Instant::now() + Duration::from_secs(120),
    );
    Ok(result(SpeechResult::Prompt {
        ticket: Some(ticket),
        number: allow.then_some(number),
    }))
}

async fn answer(
    shared: &Arc<Shared>,
    ticket: &str,
    stream: &str,
) -> Result<CommandResult, CoreError> {
    checked(shared, ticket)?;
    {
        let mut state = shared.voice_prompts.lock();
        let current = state
            .current
            .as_mut()
            .filter(|c| c.ticket == ticket && c.stream.as_deref() == Some(stream))
            .ok_or_else(refused)?;
        current.stream = None;
        current.quiet = None;
    }
    let transcript = crate::speech::stop(shared, stream).await?;
    let CommandResult::Speech {
        result: SpeechResult::Transcript { text, .. },
    } = transcript
    else {
        return Err(refused());
    };
    decide(shared, ticket, &text).await
}

async fn decide(
    shared: &Arc<Shared>,
    ticket: &str,
    text: &str,
) -> Result<CommandResult, CoreError> {
    let c = checked(shared, ticket)?;
    let Some(choice) = vw_ask::voice::answer(text, c.number, c.allow) else {
        if vw_ask::voice::repeat(text) {
            let mut state = shared.voice_prompts.lock();
            let Some(current) = state.current.as_mut().filter(|c| c.ticket == ticket) else {
                return Err(ended());
            };
            if !defer_for_reprompt(current, Instant::now()) {
                drop(state);
                clear(shared, Some(ticket));
                return Ok(status("ended"));
            }
            let deadline = current.until;
            drop(state);
            watch_deadline(shared, ticket.to_owned(), deadline);
            return Ok(status("repeat"));
        }
        let mut state = shared.voice_prompts.lock();
        if let Some(current) = state.current.as_mut().filter(|c| c.ticket == ticket) {
            current.misses += 1;
            if current.misses < 2 {
                if !defer_for_reprompt(current, Instant::now()) {
                    drop(state);
                    clear(shared, Some(ticket));
                    return Ok(status("ended"));
                }
                let deadline = current.until;
                drop(state);
                watch_deadline(shared, ticket.to_owned(), deadline);
                return Ok(missed("retry", text));
            }
        }
        drop(state);
        clear(shared, Some(ticket));
        return Ok(missed("card", text));
    };
    // Claim this exact challenge under the same lock as cancellation and session changes.
    {
        let mut state = shared.voice_prompts.lock();
        if !state.current.as_ref().is_some_and(|current| {
            current.ticket == ticket
                && state.owner == current.ask.session
                && Instant::now() < current.until
        }) {
            return Err(refused());
        }
        state.current.take();
    }
    let decision = if choice == Choice::Once {
        "approve"
    } else {
        "deny"
    };
    crate::ask::cards::answer_by(
        shared,
        Origin::Ui,
        c.ask.session.ok_or_else(refused)?,
        c.ask
            .id
            .strip_prefix("callback:")
            .ok_or_else(refused)?
            .into(),
        json!({"type":"approval","decision":{"type":decision}}),
        AnsweredBy::Voice,
    )
    .await?;
    Ok(status(if choice == Choice::Once {
        "allowed"
    } else {
        "refused"
    }))
}

pub(crate) async fn execute(
    shared: &Arc<Shared>,
    origin: Origin,
    request: SpeechRequest,
) -> Result<CommandResult, CoreError> {
    if crate::command_origin::debug_api_acts(shared, origin) {
        return Err(refused());
    }
    match request {
        SpeechRequest::PromptSession { session_id } => {
            clear(shared, None);
            if let Some(id) = &session_id {
                crate::sessions::snapshot(shared, id)?;
            }
            shared.voice_prompts.lock().owner = session_id;
            Ok(status("card"))
        }
        SpeechRequest::PromptPrepare {
            session_id,
            callback_id,
        } => prepare(shared, &session_id, &callback_id),
        SpeechRequest::PromptCancel { ticket } => {
            clear(shared, Some(&ticket));
            Ok(status("card"))
        }
        SpeechRequest::PromptStart {
            ticket,
            sample_rate,
            language,
        } => {
            let c = checked(shared, &ticket)?;
            if c.stream.is_some() {
                return Err(refused());
            }
            let quiet = if let Some(hub) = &shared.preview {
                Some(Arc::new(
                    hub.service.quiet_audio().await.map_err(|_| refused())?,
                ))
            } else {
                None
            };
            // Native silence may wait for a previous capture's restoration. Recheck this exact card afterward.
            checked(shared, &ticket)?;
            let mut started = crate::speech::start(shared, origin, sample_rate, language)?;
            if let CommandResult::Speech {
                result:
                    SpeechResult::Started {
                        stream_id,
                        answer_window_ms,
                        ..
                    },
            } = &mut started
            {
                let mut state = shared.voice_prompts.lock();
                let Some(current) = state
                    .current
                    .as_mut()
                    .filter(|c| c.ticket == ticket && c.stream.is_none())
                else {
                    shared.dictations.cancel(stream_id);
                    return Err(refused());
                };
                let window = match arm_window(current, Instant::now()) {
                    Ok(window) => window,
                    Err(error) => {
                        shared.dictations.cancel(stream_id);
                        drop(state);
                        clear(shared, Some(&ticket));
                        return Err(error);
                    }
                };
                *answer_window_ms = Some(u32::try_from(window.as_millis()).unwrap_or(u32::MAX));
                current.stream = Some(stream_id.clone());
                current.quiet = quiet;
                let deadline = current.until;
                drop(state);
                watch_deadline(shared, ticket, deadline);
            }
            Ok(started)
        }
        SpeechRequest::PromptAnswer { ticket, stream_id } => {
            answer(shared, &ticket, &stream_id).await
        }
        _ => Err(refused()),
    }
}
#[cfg(test)]
#[path = "voice_prompts_tests.rs"]
mod tests;
