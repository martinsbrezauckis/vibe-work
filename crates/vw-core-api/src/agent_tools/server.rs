//! The core's loopback listener for the `vw-agent-tools` binary: one request line per connection,
//! authenticated by credential, answered by `tools` (session credential) or hook checks (hook credential).
//!
//! Depends on: vw-agent-tools (`wire`, `credential`), tokio, `tools`, `killcheck`, `core::Shared`. Called by:
//! `core` (spawned at start, aborted at shutdown through `AgentTools::shutdown`).
//! Invariants:
//! - Bounded: one line of at most `wire::MAX_LINE` bytes, read within 10 s; at most 32 connections at once.
//! - Authentication before anything else; an unknown, revoked or pending credential gets one fixed refusal
//!   for every tool (it never learns whether a tool exists).
//! - A session credential only ever acts on its own session; the hook credential only on its named checks.
//! - Every call is logged (tool, session, outcome), never the credential or the arguments.
//! - Every answer passes `vw_secrets::scrub`: a registered stored secret never reaches the agent.

use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_agent_tools::credential::Holder;
use vw_agent_tools::wire::{self, Answer, Request, tools as names};

use crate::core::Shared;

const REFUSED: &str = "refused: unknown or expired Vibe Work credential";
// Leave time for the 50 s sidecar deadline and Vibe's 60 s MCP deadline to receive the refusal.
const BROWSER_CALL_TIMEOUT: Duration = Duration::from_secs(40);

/// Starts the accept loop on the tools' listener. `None` when the listener was already taken.
pub(crate) fn spawn_server(shared: &Arc<Shared>) -> Option<JoinHandle<()>> {
    let tools = shared.agent_tools.as_ref()?;
    let listener = tools.take_listener()?;
    if let Err(error) = listener.set_nonblocking(true) {
        warn!(%error, "agent tools listener could not be made non-blocking");
        return None;
    }
    let listener = match TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            warn!(%error, "agent tools listener could not be registered");
            return None;
        }
    };
    let weak = Arc::downgrade(shared);
    Some(tokio::spawn(accept_loop(listener, weak)))
}

async fn accept_loop(listener: TcpListener, shared: Weak<Shared>) {
    let limit = Arc::new(Semaphore::new(32));
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            // Out of descriptors or a transient error: back off instead of spinning.
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let _permit = permit;
            serve_one(stream, shared).await;
        });
    }
}

async fn serve_one(stream: TcpStream, shared: Weak<Shared>) {
    let (read, mut write) = stream.into_split();
    let limit = u64::try_from(wire::MAX_LINE + 1).unwrap_or(u64::MAX);
    let mut reader = BufReader::new(read.take(limit));
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await;
    let (answer, browser_answer) = match read {
        Ok(Ok(n)) if n > 0 && n <= wire::MAX_LINE && line.ends_with('\n') => {
            match serde_json::from_str::<Request>(&line) {
                Ok(request) => {
                    let browser_answer = names::BROWSER_TOOLS.contains(&request.tool.as_str());
                    let answer = if browser_answer {
                        // The socket owns this browser call. Dropping its future on EOF or timeout
                        // also drops a pending native permission wait and its action guard.
                        let Some(answer) = browser_answer_while_connected(
                            &mut reader,
                            Box::pin(answer(&shared, request)),
                            BROWSER_CALL_TIMEOUT,
                        )
                        .await
                        else {
                            return;
                        };
                        answer
                    } else if request.tool == names::GIT_CHECK {
                        // A hook that has exited can no longer use this approval. Release its
                        // confirmation future and listener slot as soon as the client goes away.
                        let Some(answer) =
                            answer_while_connected(&mut reader, answer(&shared, request)).await
                        else {
                            return;
                        };
                        answer
                    } else {
                        answer(&shared, request).await
                    };
                    (answer, browser_answer)
                }
                Err(_) => (Answer::Error("bad request".to_owned()), false),
            }
        }
        _ => (Answer::Error("bad request".to_owned()), false),
    };
    if let Ok(text) = serde_json::to_vec(&answer) {
        // No stored secret leaves through an agent tool answer, in any form `vw-secrets` searches
        // (a `secret_run`'s output, a page that shows a stored key, a process's output). The marker is plain ASCII
        // without quotes, so the line stays valid JSON.
        let mut text = match vw_secrets::scrub(&text) {
            Some((clean, kinds)) => {
                tracing::info!(
                    kinds = ?kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                    "agent tool answer: stored secrets withheld"
                );
                clean
            }
            None => text,
        };
        // Stored values keep their established kind marker; then one browser-specific pass covers all answer
        // fields, including receipts and errors. A malformed scrubbed answer fails closed.
        if browser_answer {
            let mut filtered = serde_json::from_slice::<Answer>(&text)
                .unwrap_or_else(|_| Answer::Error("browser answer withheld".to_owned()));
            match &mut filtered {
                Answer::Ok(value) => vw_browser::filter_agent_output(value),
                Answer::Error(reason) => {
                    let mut value = Value::String(std::mem::take(reason));
                    vw_browser::filter_agent_output(&mut value);
                    value.as_str().unwrap_or_default().clone_into(reason);
                }
            }
            text = serde_json::to_vec(&filtered)
                .unwrap_or_else(|_| b"{\"error\":\"browser answer withheld\"}".to_vec());
        }
        text.push(b'\n');
        let _ = write.write_all(&text).await;
        let _ = write.shutdown().await;
    }
}

async fn browser_answer_while_connected<R, F>(
    reader: &mut R,
    answer: F,
    deadline: Duration,
) -> Option<Answer>
where
    R: AsyncRead + Unpin,
    F: Future<Output = Answer>,
{
    answer_while_connected(reader, async {
        tokio::time::timeout(deadline, answer).await.unwrap_or_else(|_| {
            Answer::Error("Outcome unknown: the browser request timed out and was canceled. Do not repeat an act; inspect the Browser tab. An unanswered native confirmation can no longer send input.".to_owned())
        })
    }).await
}

async fn answer_while_connected<R, F>(reader: &mut R, answer: F) -> Option<Answer>
where
    R: AsyncRead + Unpin,
    F: Future<Output = Answer>,
{
    let mut extra = [0_u8; 1];
    tokio::select! {
        biased;
        // Any further input violates the one-request protocol; EOF means the caller has gone.
        _ = reader.read(&mut extra) => None,
        result = answer => Some(result),
    }
}

async fn answer(shared: &Weak<Shared>, request: Request) -> Answer {
    let Some(shared) = shared.upgrade() else {
        return Answer::Error("Vibe Work is shutting down".to_owned());
    };
    let Some(tools) = shared.agent_tools.clone() else {
        return Answer::Error(REFUSED.to_owned());
    };
    let holder = tools.table.holder(&request.credential);
    let tool = request.tool.as_str();
    let outcome = match holder {
        Some(Holder::Session(session_id)) if names::SESSION_TOOLS.contains(&tool) => {
            let outcome =
                super::tools::call(&shared, &tools, &session_id, tool, request.args).await;
            log(tool, Some(&session_id), &outcome);
            outcome
        }
        Some(Holder::Hook) if tool == names::KILL_CHECK => {
            let outcome = super::killcheck::check(&shared, request.args).await;
            log(tool, None, &outcome);
            outcome
        }
        Some(Holder::Hook) if tool == names::GIT_CHECK => {
            let outcome = super::gitcheck::check(&shared, request.args).await;
            log(tool, None, &outcome);
            outcome
        }
        // may this session use another copy (one of its family tree of subagents)?
        Some(Holder::Hook) if tool == names::COPY_CHECK => {
            let outcome = super::copycheck::check(&shared, request.args).await;
            log(tool, None, &outcome);
            outcome
        }
        Some(Holder::Hook) if tool == names::CONNECTOR_CHECK => {
            let name = request
                .args
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("");
            Ok(serde_json::json!({"removed": shared.connectors.removed_tool(name)}))
        }
        Some(Holder::Session(_) | Holder::Hook) => {
            log(tool, None, &Err(String::new()));
            Err(format!(
                "refused: this credential may not call {}",
                short(tool)
            ))
        }
        Some(Holder::Pending) | None => {
            log(tool, None, &Err(String::new()));
            Err(REFUSED.to_owned())
        }
    };
    match outcome {
        Ok(value) => Answer::Ok(value),
        Err(reason) => Answer::Error(reason),
    }
}

fn short(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect()
}

fn log(tool: &str, session: Option<&str>, outcome: &Result<Value, String>) {
    info!(
        tool = short(tool),
        session = session.unwrap_or("-"),
        ok = outcome.is_ok(),
        "agent tool call"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Cancelled(Arc<AtomicBool>);

    impl Drop for Cancelled {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn browser_call_is_dropped_when_caller_disconnects() {
        let (mut server, client) = tokio::io::duplex(64);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let pending = async move {
            let _guard = Cancelled(flag);
            let _ = started.send(());
            std::future::pending::<Answer>().await
        };
        let task = tokio::spawn(async move {
            browser_answer_while_connected(&mut server, pending, Duration::from_secs(1)).await
        });
        ready.await.unwrap();
        drop(client);
        assert!(task.await.unwrap().is_none());
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn browser_call_has_a_bounded_confirmation_wait() {
        let (mut server, _client) = tokio::io::duplex(64);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let pending = async move {
            let _guard = Cancelled(flag);
            std::future::pending::<Answer>().await
        };
        let result =
            browser_answer_while_connected(&mut server, pending, Duration::from_millis(20)).await;
        assert_eq!(
            result,
            Some(Answer::Error(
                "Outcome unknown: the browser request timed out and was canceled. Do not repeat an act; inspect the Browser tab. An unanswered native confirmation can no longer send input.".to_owned()
            ))
        );
        assert!(cancelled.load(Ordering::SeqCst));
    }
}
