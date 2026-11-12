//! Event-time confirmation for checked native clicks; the browser polls only the isolated frame seam.
//! Records failures in the ordinary action receipt; a submitted click can never become a pre-input refusal.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::driver::{ActError, failed};
use serde_json::Value;
use vw_preview::FrameAnswer;

pub(crate) fn next_token() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed).to_string()
}

/// Collects only the blocked bit after dispatch admission failed (for example the person clicked during the
/// mark). No native lease exists then. Never reads page labels across frame policy boundaries; always disarms
/// this receipt so the person's next click works immediately.
pub(crate) async fn cancelled(
    service: &vw_preview::PreviewService,
    mount: &vw_preview::MountId,
    (page, token): (&str, &str),
    mut diagnosis: ActError,
) -> ActError {
    let script = format!(
        "(() => {{ const req = {}; const state = window[req.page + '_pointReceipt']; if (!state || state.token !== req.token) return false; const blocked = Boolean(state.result.blocked); state.clear(); return blocked; }})()",
        serde_json::json!({"page":page,"token":token})
    );
    let Ok(Some(answers)) = service
        .evaluate_frames_on_mount(mount, None, script, None)
        .await
    else {
        return diagnosis;
    };
    if answers.iter().any(|answer| {
        let raw: Value = serde_json::from_str(&answer.value).unwrap_or(Value::Null);
        raw == true || raw.as_str() == Some("true")
    }) {
        diagnosis.text = "the page moved: native input was blocked; read the page again".into();
    }
    diagnosis
}

/// How long the page's mouse events normally take to reach the receipt; no new read starts after it.
const WAIT: Duration = Duration::from_millis(1000);
/// A read already in flight at [`WAIT`] is never cut short before this: its answer may hold the diagnosis.
const READ_LIMIT: Duration = Duration::from_secs(5);

pub(crate) struct Confirmation {
    pub scroll: Option<Value>,
    pub effect: Option<String>,
}

/// Confirms one dispatched native click from its event-time receipt. The input is never repeated. Whatever was
/// observed decides the answer, also when the receipt is delivered late: a read in flight at the deadline is
/// awaited, and the final read-and-clear (always made) is interpreted too, so a misdirected or overlapping click
/// is reported with its reason instead of "could not be confirmed".
pub(crate) async fn confirm_events(
    input: &vw_preview::NativeClickLease<'_>,
    page: &str,
    token: &str,
    frame: &str,
    document: &str,
    (options, expected): (vw_preview::ClickOptions, &[&str]),
) -> Result<Confirmation, ActError> {
    let read = |clear: bool| async move {
        input
            .evaluate_frames(crate::scripts::point_receipt(page, token, clear))
            .await
            .map_err(|e| failed(format!("native click target could not be confirmed: {e}")))
    };
    let deadline = tokio::time::Instant::now() + WAIT;
    let polled = async {
        loop {
            if complete_events(
                &read(false).await?,
                token,
                frame,
                document,
                options,
                expected,
            )? {
                return Ok(true);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let polled = tokio::time::timeout(WAIT + READ_LIMIT, polled)
        .await
        .unwrap_or(Ok(false));
    // Keep listeners through the whole action. Only the final read-and-clear can confirm it; another trusted
    // sequence between a poll and collection must still fail.
    let last = read(true).await;
    polled?;
    // A final read that failed says why (for example the page's frames changed while clicking).
    match complete_events(
        last.as_ref().map_err(Clone::clone)?,
        token,
        frame,
        document,
        options,
        expected,
    ) {
        Ok(true) => {
            let value = last?
                .iter()
                .find_map(|answer| {
                    let raw: Value = serde_json::from_str(&answer.value).ok()?;
                    let value = raw
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(raw);
                    (answer.id == frame && value["token"] == token).then_some(value)
                })
                .unwrap_or(Value::Null);
            Ok(Confirmation {
                scroll: value["scroll"].is_object().then(|| value["scroll"].clone()),
                effect: value["effect"].as_str().map(str::to_owned),
            })
        }
        Ok(false) => Err(unconfirmed()),
        Err(diagnosis) => Err(diagnosis),
    }
}

fn unconfirmed() -> ActError {
    failed("native input was sent; its event-time target could not be confirmed")
}

fn ambiguous() -> ActError {
    failed(
        "native input was sent; the page or the user clicked at the same time, so earlier refs may no longer match; read the page again",
    )
}

fn complete_events(
    answers: &[FrameAnswer],
    token: &str,
    frame: &str,
    document: &str,
    options: vw_preview::ClickOptions,
    expected: &[&str],
) -> Result<bool, ActError> {
    let receipts: Vec<_> = answers
        .iter()
        .filter_map(|answer| {
            let raw: Value = serde_json::from_str(&answer.value).unwrap_or(Value::Null);
            let value: Value = if let Some(text) = raw.as_str() {
                serde_json::from_str(text).unwrap_or(Value::Null)
            } else {
                raw
            };
            (value["token"] == token).then_some((answer, value))
        })
        .collect();
    for (_, value) in &receipts {
        if value["blocked"].is_object() {
            let role = safe_label(&value["blocked"]["role"], "element");
            let name = safe_label(&value["blocked"]["name"], "");
            return Err(failed(format!(
                "the page moved: blocked native input on {role} {name}; read the page again"
            )));
        }
    }
    // Check overlap across every frame before interpreting target labels, including a second sequence
    // delivered to a different frame. Receipt storage is bounded, with a sticky ambiguity flag on overflow.
    let count: usize = receipts
        .iter()
        .filter_map(|(_, value)| value["events"].as_array())
        .map(Vec::len)
        .sum();
    if count > expected.len() || receipts.iter().any(|(_, v)| v["ambiguous"] == true) {
        return Err(ambiguous());
    }
    let mut confirmed = false;
    for (answer, value) in receipts {
        let Some(events) = value["events"].as_array() else {
            continue;
        };
        let Some(dispatch) = value["dispatchedAt"].as_f64().filter(|v| v.is_finite()) else {
            continue;
        };
        if answer.id == frame
            && value["document"] == document
            && value["ready"] == true
            && value["rejected"] == true
            && expected.first() == Some(&"beforeinput")
        {
            return Err(failed(
                "The field does not contain the requested text; the page canceled native text entry. Read the field before another action.",
            ));
        }
        for (index, event) in events.iter().enumerate() {
            if !event["timeStamp"]
                .as_f64()
                .is_some_and(|time| time.is_finite() && time > dispatch)
                || expected
                    .get(index)
                    .is_none_or(|kind| event["type"] != *kind)
            {
                return Err(ambiguous());
            }
            if answer.id != frame || value["document"] != document || event["matches"] != true {
                let role = safe_label(&event["role"], "element");
                let name = safe_label(&event["name"], "");
                return Err(failed(format!(
                    "the page moved: the click landed on {role} {name} (trusted {} receipt, frame {})",
                    event["type"].as_str().unwrap_or("mouse"),
                    answer.id
                )));
            }
        }
        confirmed |= answer.id == frame
            && value["document"] == document
            && events.len() == expected.len()
            && ((options.wheel.is_none() && expected.first() != Some(&"beforeinput"))
                || value["ready"] == true);
    }
    Ok(confirmed)
}

fn safe_label(value: &Value, fallback: &str) -> String {
    let text = value.as_str().unwrap_or(fallback);
    if crate::guard::instructs(text) {
        return "[withheld page text]".into();
    }
    text.chars().filter(|c| !c.is_control()).take(160).collect()
}
