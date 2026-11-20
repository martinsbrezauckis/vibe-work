//! The helper connection's lifecycle: start, supervision with backoff, serving one pipe, and the startup synchronization.
use super::{
    Answer, Arc, Call, Duration, PathBuf, PoisonError, Running, Shared, UNKNOWN, Value, Weak, json,
    mapping, mpsc, oneshot, own, publish, setup, update_pairing, update_work, watch,
};

pub(super) async fn start_inner(shared: &Arc<Shared>, removing: bool) -> Result<(), String> {
    let chrome = &shared.chrome_control;
    if !removing && !chrome.any_enabled() && !super::super::safari::enabled(shared) {
        return Ok(());
    }
    let Some(dir) = chrome.dir() else {
        return Ok(());
    };
    let configured = setup::view(&chrome.config, Some(dir), &shared.instructions.own_home).state
        != setup::SetupState::NotSetUp
        || setup::registered(&chrome.config, Some(dir), "Microsoft Edge")
        || (cfg!(feature = "your-firefox")
            && (dir.join("firefox-window").is_file()
                || setup::registered(&chrome.config, Some(dir), "Mozilla Firefox")))
        || super::super::safari::enabled(shared);
    if !configured {
        return Ok(());
    }
    if chrome
        .owner
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
    {
        return Ok(());
    }
    let keys = shared
        .config
        .keys
        .as_ref()
        .ok_or(vw_vocabulary::texts::CHROME_NEEDS_KEYRING)?;
    // Without a system keyring (the app's fallback store) the identity cannot be read at all: that is named with its
    // next step rather than reported as a missing credential.
    let secret = keys.chrome_owner().await;
    chrome.owner.keyring_unavailable.store(
        secret.as_ref().is_err_and(setup::keyring_unavailable),
        std::sync::atomic::Ordering::SeqCst,
    );
    if secret.is_err() {
        publish(shared);
    }
    let secret = secret.map_err(|error| setup::owner_read_failed(&error))?;
    chrome
        .owner
        .credential_missing
        .store(secret.is_none(), std::sync::atomic::Ordering::SeqCst);
    if secret.is_none() {
        publish(shared);
    }
    let secret = secret.ok_or(vw_vocabulary::texts::CHROME_OWNER_MISSING)?;

    let (_, bytes) = setup::packaged_helper(&chrome.config)?;
    let expected = blake3::hash(&bytes);
    let package = chrome.config.clone();
    let (send, requests) = mpsc::channel(32);
    let (stop, stopped) = watch::channel(false);
    let weak = Arc::downgrade(shared);
    let dir = dir.to_owned();
    let task = tokio::spawn(supervise(
        weak, package, dir, expected, secret, requests, stopped,
    ));
    let mut running = chrome
        .owner
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if running.is_some() {
        let _ = stop.send(true);
    } else {
        *running = Some(Running { send, stop, task });
    }
    Ok(())
}

pub(super) async fn supervise(
    shared: Weak<Shared>,
    package: setup::ChromeControlConfig,
    dir: PathBuf,
    expected: blake3::Hash,
    secret: vw_vault::SecretString,
    mut requests: mpsc::Receiver<Call>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = 100;
    while !*stop.borrow() {
        let result = tokio::select! {
            _ = stop.changed() => return,
            result = super::super::pipe::Pipe::launch(&package, &dir, expected, &secret, shared.clone()) => result,
        };
        let conflict = matches!(&result, Err(error) if error.helper_code == Some(super::super::pipe::OWNER_CONFLICT));
        if let Some(core) = shared.upgrade() {
            let before = core
                .chrome_control
                .owner
                .helper_conflict
                .swap(conflict, std::sync::atomic::Ordering::SeqCst);
            if conflict && !before {
                // Logged once per refusal streak; the retries continue with backoff.
                tracing::warn!(
                    "Chrome Control helper refused: another Vibe Work browser helper holds this connection"
                );
            }
        }
        if let Err(error) = &result
            && !conflict
        {
            tracing::debug!(code = ?error.helper_code, "Chrome Control helper did not start or was refused");
        }
        if let Ok(mut pipe) = result {
            if let Some(shared) = shared.upgrade() {
                shared
                    .chrome_control
                    .tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            }
            serve_connection(&shared, &mut pipe, &mut requests, &mut stop).await;
            if *stop.borrow() {
                // Stop or quit: the helper closes its Firefox window before it ends.
                pipe.finish().await;
            } else {
                pipe.abort().await;
            }
        }
        if let Some(core) = shared.upgrade() {
            core.chrome_control.owner.clear_connection();
            super::super::profiles::disconnected(&core.chrome_control);
            core.chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .disconnected();
            core.chrome_control
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            publish(&core);
        }
        if *stop.borrow() {
            return;
        }
        // Refuse queued and new work for the whole backoff; traffic cannot shorten it or replay work.
        let until = tokio::time::Instant::now() + Duration::from_millis(backoff);
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                () = tokio::time::sleep_until(until) => break,
                call = requests.recv() => {
                    if let Some(call) = call { let _ = call.reply.send(own(vw_vocabulary::texts::CHROME_RESTARTING, "OWNER_CLOSED")); }
                    else { return; }
                }
            }
        }
        backoff = (backoff * 2).min(5000);
    }
}

/// Synchronizes and serves one pipe; every pending caller is settled before the owner disconnects it.
pub(super) async fn serve_connection(
    shared: &Weak<Shared>,
    pipe: &mut super::super::pipe::Pipe,
    requests: &mut mpsc::Receiver<Call>,
    stop: &mut watch::Receiver<bool>,
) {
    let mut sent_policy = Value::Null;
    let synchronized = tokio::select! {
        _ = stop.changed() => false,
        result = synchronize(shared, pipe, &mut sent_policy) => result.is_ok(),
    };
    if !synchronized {
        // Failed startup needs the same disconnect, queue rejection and backoff as a lost pipe.
        return;
    }
    let mut pending = std::collections::HashMap::<
        u64,
        (Option<oneshot::Sender<Answer>>, tokio::time::Instant),
    >::new();
    let mut poll = tokio::time::interval(Duration::from_secs(2));
    // Diagnostics: why this connection ended (a fixed word; no helper or page text).
    let mut ended = "stopped";
    loop {
        let expiry = pending
            .values()
            .map(|(_, start)| *start + Duration::from_secs(35))
            .min();
        tokio::select! {
            biased;
            _ = stop.changed() => { break; }
            reply = pipe.next() => {
                let Ok((id, result)) = reply else { ended = "helper pipe closed"; break };
                let Some((reply, started)) = pending.remove(&id) else { ended = "unexpected reply"; break };
                update_pairing(shared, &result);
                update_work(shared, &result, started);
                if let Some(reply) = reply { let _ = reply.send(Answer::Helper(result)); }
            }
            call = requests.recv() => {
                let Some(call) = call else { ended = "requests closed"; break };
                if !call.admitted() { continue; }
                if pending.len() >= 32 && call.name != "chrome_session" {
                    let _ = call.reply.send(own("Not run: browser control is busy. Retry this browser action after a moment.", "BUSY"));
                    continue;
                }
                let begun = if call.rpc {
                    pipe.begin_tolerant(&call.name, call.args).await
                } else {
                    pipe.begin("tools/call", json!({"name":call.name,"arguments":call.args})).await
                };
                if let Ok(id) = begun {
                    pending.insert(id, (Some(call.reply), tokio::time::Instant::now()));
                } else { let _ = call.reply.send(own(UNKNOWN, "TRANSPORT_LOST")); ended = "send failed"; break; }
            }
            _ = poll.tick(), if !pending.values().any(|(reply, _)| reply.is_none()) => {
                if let Ok(id) = pipe.begin("tools/call", json!({"name":"chrome_status","arguments":{}})).await {
                    pending.insert(id, (None, tokio::time::Instant::now()));
                } else { ended = "status poll send failed"; break }
            }
            () = async {
                if let Some(deadline) = expiry {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                ended = "a call outlived 35 s"; break;
            }
        }
    }
    tracing::warn!(reason = ended, "Chrome Control connection ended");
    for (_, (reply, _)) in pending {
        if let Some(reply) = reply {
            let _ = reply.send(own(UNKNOWN, "TRANSPORT_LOST"));
        }
    }
}

pub(super) async fn synchronize(
    shared: &Weak<Shared>,
    pipe: &mut super::super::pipe::Pipe,
    sent: &mut Value,
) -> Result<(), ()> {
    // Read trusted lowerings before sending any policy. An unchanged policy is never re-applied over one.
    pipe.notify("notifications/owner/drain", json!({})).await?;
    pipe.rpc("ping", json!({})).await?;
    super::super::switches::synchronize(shared.upgrade().ok_or(())?.as_ref(), pipe).await?;
    let next = mapping::policy();
    if *sent != next {
        pipe.notify("notifications/owner/policy", next.clone())
            .await?;
        pipe.rpc("ping", json!({})).await?;
        *sent = next;
    }
    Ok(())
}
