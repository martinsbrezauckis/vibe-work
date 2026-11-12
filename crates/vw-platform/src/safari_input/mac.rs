//! Dedicated native-input worker retains AX window tickets on one thread, with cancellation before every effect.
#[path = "mac_ax.rs"]
mod ax;
#[path = "mac_events.rs"]
mod events;
use super::geometry;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
struct Request {
    stage: String,
    prepared: Value,
    action: String,
    args: Value,
    url: String,
    cancel: Arc<AtomicBool>,
    reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
}
struct Ticket {
    window: ax::Window,
    created: Instant,
}
static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
pub(super) async fn request(
    stage: &str,
    prepared: Value,
    action: &str,
    args: Value,
    url: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Value, String> {
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Request>(8);
        std::thread::spawn(move || {
            let mut tickets = HashMap::<String, Ticket>::new();
            while let Ok(r) = rx.recv() {
                tickets.retain(|_, t| t.created.elapsed() < Duration::from_secs(8));
                if r.reply.is_closed() || r.cancel.load(Ordering::Acquire) {
                    continue;
                }
                let result = run(&r, &mut tickets);
                let _ = r.reply.send(result);
            }
        });
        tx
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = Request {
        stage: stage.into(),
        prepared,
        action: action.into(),
        args,
        url: url.into(),
        cancel,
        reply: tx,
    };
    worker.try_send(request).map_err(|_| {
        "Safari native input is busy. Wait for the current action before trying again.".to_owned()
    })?;
    rx.await.map_err(|_| {
        "The Safari input worker ended. Reconnect Safari before continuing.".to_owned()
    })?
}
fn check(r: &Request) -> Result<(), String> {
    if r.reply.is_closed() || r.cancel.load(Ordering::Acquire) {
        Err("Safari input was cancelled. Read the current page before continuing.".into())
    } else {
        Ok(())
    }
}
fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|_| {
        "Safari's native preparation is malformed. Read the page and prepare a new action.".into()
    })
}
fn run(r: &Request, tickets: &mut HashMap<String, Ticket>) -> Result<Value, String> {
    check(r)?;
    if r.stage == "prepare" {
        let bounds = parse(&r.prepared["window"])?;
        let window = super::lifecycle::prepare(
            || ax::Window::discover(bounds),
            || check(r),
            |window, live| window.activate(live),
        )?;
        check(r)?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|_| "Native input could not create a ticket. Try a new action.".to_owned())?;
        let id = bytes.iter().fold(String::with_capacity(32), |mut id, b| {
            let _ = std::fmt::Write::write_fmt(&mut id, format_args!("{b:02x}"));
            id
        });
        tickets.insert(
            id.clone(),
            Ticket {
                window,
                created: Instant::now(),
            },
        );
        return Ok(json!({"nativeTicket":id,"safariBroughtToFront":true}));
    }
    let id = r.prepared["nativeTicket"]
        .as_str()
        .ok_or("Native input has no preparation ticket. Prepare a new action.")?;
    let ticket = tickets.remove(id).ok_or(
        "Native input's preparation expired or was used. Read the page and prepare a new action.",
    )?;
    if r.stage != "dispatch" || ticket.created.elapsed() > Duration::from_secs(8) {
        return Err("The native input preparation expired. Read the page again.".into());
    }
    let checked = r.prepared["checkedAt"]
        .as_u64()
        .ok_or("Native input has no recent target check. Read the page again.")?;
    let viewport = ticket.window.viewport(&r.url)?;
    let css = parse(&r.prepared["viewport"])?;
    let point = geometry::map(
        ticket.window.bounds(),
        viewport,
        css,
        parse(&r.prepared["point"])?,
    )?;
    let end = if r.action == "page.drag" {
        Some(geometry::map(
            ticket.window.bounds(),
            viewport,
            css,
            parse(&r.prepared["end"])?,
        )?)
    } else {
        None
    };
    check(r)?;
    ticket.window.front()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "The system clock is unavailable. Retry after checking the clock.")?
        .as_millis();
    geometry::recent(checked, now)?;
    events::dispatch(&r.action, &r.args, point, end, || {
        super::lifecycle::confirm(
            || check(r),
            || {
                ticket.window.front()?;
                if let Some(expected) = r.prepared["clipboardRevision"].as_i64()
                    && expected
                        != objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as i64
                {
                    return Err("The clipboard changed after its vault check. Inspect the clipboard and prepare a new Paste action.".into());
                }
                Ok(())
            },
        )
    })?;
    Ok(
        json!({"nativeInputPosted":true,"trustedInput":true,"safariBroughtToFront":true,"realCursorMoved":true,"instruction":"Safari was brought to the front and the real cursor moved. Inspect the extension's page readback before another action."}),
    )
}
