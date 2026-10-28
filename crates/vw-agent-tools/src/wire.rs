//! The wire between the `vw-agent-tools` binary and the Vibe Work core: one TCP connection to the core's
//! loopback listener per request, one JSON line each way.
//!
//! Request: `{"credential": "<hex>", "tool": "<name>", "args": {...}}`. Answer: `{"ok": <json>}` or
//! `{"error": "<plain text reason>"}`. A short-lived connection per call keeps nothing to resynchronize after
//! a core restart and makes every call carry its credential.
//!
//! Depends on: serde, `serde_json`, std (`TcpStream`). Called by: `mcp`, `hook`, vw-core-api (server side),
//! tests.
//! Invariants:
//! - Lines are bounded (requests [`MAX_LINE`], answers [`MAX_ANSWER`]); a longer line is an error, never buffered
//!   without limit.
//! - The credential is never part of an error text.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Environment variable naming the core's listener (`127.0.0.1:<port>`) for the MCP server.
pub const ENV_ENDPOINT: &str = "VW_AGENT_ENDPOINT";
/// Environment variable carrying the session's credential for the MCP server.
pub const ENV_CREDENTIAL: &str = "VW_AGENT_CREDENTIAL";
/// Core-supplied launch argument for a build without the Chrome package.
pub const WITHOUT_CHROME_ARG: &str = "--without-chrome";

/// Advertise the consented Chrome tool catalog.
pub const CHROME_CATALOG_ARG: &str = "--chrome-catalog";
/// Name of the per-session MCP server in `agentConfig.mcpServers` (the agent sees `mcp_vw.<tool>`).
pub const SERVER_NAME: &str = "vw";
/// Longest request line in bytes (and longest answer of every tool except a screenshot).
pub const MAX_LINE: usize = 64 * 1024;
/// Twenty 1 MiB text files, including worst-case JSON escaping and the envelope.
pub const MAX_PROPOSAL_LINE: usize = 121 * 1024 * 1024;
/// Longest answer line in bytes: a `preview_screenshot` (and a `browser_read` with `screenshot`) answer carries a PNG
/// of at most 1 MiB as base64, so answers may be larger than requests.
pub const MAX_ANSWER: usize = 2 * 1024 * 1024;
/// Key of an image in a tool's JSON answer (`{"data": <base64>, "mimeType": "image/png"}`): the MCP server turns
/// it into an MCP image content block instead of text.
pub const IMAGE_KEY: &str = "vwImage";
/// How long the binary waits for the core's answer. Under Vibe's fixed 60 s MCP tool timeout, above
/// the bounded start/restart reply (including slow OS scans), with a transport margin.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(50);
/// Prefix for a lost reply to a process mutation: the request may have run, so it is never a refusal.
pub const UNKNOWN_OUTCOME: &str = "Outcome unknown:";

/// Plain refusal for a tool absent from this build.
pub const CHROME_UNAVAILABLE: &str = "Chrome Control is not in this version of Vibe Work.";

/// The session-scoped tools (MCP names).
pub mod tools {
    /// Bind a Vibe source/call to one transport receipt (hook credential only).
    pub const TOOL_RECEIPT: &str = "tool_receipt";
    /// Hook-injected, single-use transport field; overwritten before each MCP call.
    pub const RECEIPT_FIELD: &str = "_vwReceipt";
    /// Reports one final hook refusal to the core bell. Hook credential only.
    pub const GUARD_REFUSED: &str = "guard_refused";
    /// Hook-only observation of the exact command before Vibe's display projection.
    pub const SHELL_COMMAND: &str = "shell_command";
    /// This session's processes, listeners and leased ports.
    pub const PROCESSES_LIST: &str = "processes_list";
    /// This session's leased ports.
    pub const PORT_LEASE: &str = "port_lease";
    /// Start a long-running process (dev server) that survives the app.
    pub const PROCESS_START: &str = "process_start";
    /// Stop one of this session's processes.
    pub const PROCESS_STOP: &str = "process_stop";
    /// Restart a process started with `process_start`.
    pub const PROCESS_RESTART: &str = "process_restart";
    /// The kill guard's question (hook credential only).
    pub const KILL_CHECK: &str = "kill_check";
    /// The git guard's question (hook credential only).
    pub const GIT_CHECK: &str = "git_check";
    /// The folder guard's question (hook credential only): may this session work in that copy now (one
    /// active session per copy)?
    pub const COPY_CHECK: &str = "copy_check";
    /// The connector hook's question: was this MCP server removed while an open chat still lists its tool?
    pub const CONNECTOR_CHECK: &str = "connector_check";
    /// The Chrome Control guard's question (hook credential only): may this Chrome Control call run? The core
    /// answers at once, or after the user answered its card in the session.
    pub const CHROME_CHECK: &str = "chrome_check";
    /// This session's preview and the URLs it may open.
    pub const PREVIEW_LIST: &str = "preview_list";
    /// Open a URL of this session in its preview.
    pub const PREVIEW_OPEN: &str = "preview_open";
    /// A PNG of this session's preview.
    pub const PREVIEW_SCREENSHOT: &str = "preview_screenshot";
    /// Recent console errors and warnings of the preview's page.
    pub const PREVIEW_CONSOLE: &str = "preview_console";
    /// Validate this session's project handbook (Handbook Format 1.1 §10). Reads files only.
    pub const HANDBOOK_CHECK: &str = "handbook_check";
    /// The user's answers to the inputs of this project's handbook pages (Format 1.2 §6.6). Reads only.
    pub const HANDBOOK_ANSWERS: &str = "handbook_answers";
    /// Lay out a handbook page at desktop and phone width in light and dark and report what is wrong.
    pub const HANDBOOK_VISUAL_CHECK: &str = "handbook_visual_check";
    /// The handbook tools (the core runs them for the credential's session project).
    pub const HANDBOOK_TOOLS: [&str; 3] = [HANDBOOK_CHECK, HANDBOOK_ANSWERS, HANDBOOK_VISUAL_CHECK];
    /// Read the page in this session's built-in browser: refs, text, prompt-guard flags.
    pub const BROWSER_READ: &str = "browser_read";
    /// Act on the page in this session's built-in browser and get a receipt.
    pub const BROWSER_ACT: &str = "browser_act";
    /// Capture from the page in this session's built-in browser into the session folder.
    pub const BROWSER_CAPTURE: &str = "browser_capture";
    /// Explains that saving a page key to the vault is user-only; always refuses without accessing the page.
    pub const BROWSER_CAPTURE_SECRET: &str = "browser_capture_secret";
    /// The browser tools.
    pub const BROWSER_TOOLS: [&str; 4] = [
        BROWSER_READ,
        BROWSER_ACT,
        BROWSER_CAPTURE,
        BROWSER_CAPTURE_SECRET,
    ];
    /// The secrets the agent may ask to use (names only).
    pub const SECRET_LIST: &str = "secret_list";
    /// Run one program with secrets in its environment, after the user's consent.
    pub const SECRET_RUN: &str = "secret_run";
    /// The state or result of a `secret_run` that was still waiting or running.
    pub const SECRET_RUN_STATUS: &str = "secret_run_status";
    /// The secret broker's tools.
    pub const SECRET_TOOLS: [&str; 3] = [SECRET_LIST, SECRET_RUN, SECRET_RUN_STATUS];
    /// The libraries this session's agent may search, with their file names.
    pub const LIBRARIES_LIST: &str = "libraries_list";
    /// Ask across those libraries; an answer with numbered sources.
    pub const LIBRARY_SEARCH: &str = "library_search";
    /// Make or edit an image with Mistral, saved in the session folder.
    pub const IMAGE_GENERATE: &str = "image_generate";
    /// Read a saved attachment using Vibe Work's document reader.
    pub const DOCUMENT_READ: &str = "document_read";
    /// The work tools (the core runs them with the user's Mistral key).
    pub const WORK_TOOLS: [&str; 3] = [LIBRARIES_LIST, LIBRARY_SEARCH, IMAGE_GENERATE];
    /// The preview tools.
    pub const PREVIEW_TOOLS: [&str; 4] = [
        PREVIEW_LIST,
        PREVIEW_OPEN,
        PREVIEW_SCREENSHOT,
        PREVIEW_CONSOLE,
    ];
    /// Read this session's previous reply aloud once in the app window.
    pub const READ_ALOUD: &str = "read_aloud";
    /// What the user has open beside the chat (editor tabs, unsaved text, selection, the Files tab). Reads only.
    pub const PANEL_READ: &str = "panel_read";
    /// Capture protected-folder changes and wait (bounded) for the owner review.
    pub const PROPOSE_PROTECTED_CHANGE: &str = "propose_protected_change";
    /// The outcome of a proposal whose review was still open when `propose_protected_change` answered.
    pub const PROTECTED_CHANGE_STATUS: &str = "protected_change_status";
    /// The protected-folder review tools.
    pub const PROTECTED_CHANGE_TOOLS: [&str; 2] =
        [PROPOSE_PROTECTED_CHANGE, PROTECTED_CHANGE_STATUS];
    /// Turn Chrome Control's tools on for this chat (only when the user asks for their Chrome).
    pub const USE_YOUR_CHROME: &str = "use_your_chrome";
    /// Turn the headless browser plugin's tools on for this chat (developers only, a separate install).
    pub const USE_HEADLESS_BROWSER: &str = "use_headless_browser";
    /// Why a Chrome Control step of this chat was not done, in Vibe Work's own words (inside `run_typescript`
    /// Vibe 2.26.1 gives the agent only "Tool execution was skipped by Runtime policy" for a refused step).
    pub const CHROME_OUTCOME: &str = "chrome_outcome";
    /// Private discovery request: returns only this chat's helper tool schemas.
    pub const CHROME_TOOL_LIST: &str = "chrome_tool_list";
    /// Browser tools forwarded through the checked core gateway.
    pub const CHROME_TOOLS: [&str; 3] = ["chrome_status", "chrome_task", "chrome_execute"];
    /// Every Chrome entry point, including private discovery, shares the package gate.
    #[must_use]
    pub fn is_chrome(name: &str) -> bool {
        matches!(name, USE_YOUR_CHROME | CHROME_OUTCOME | CHROME_TOOL_LIST)
            || CHROME_TOOLS.contains(&name)
    }

    /// Every tool a session credential may call.
    pub const SESSION_TOOLS: [&str; 33] = [
        PROCESSES_LIST,
        PORT_LEASE,
        PROCESS_START,
        PROCESS_STOP,
        PROCESS_RESTART,
        PREVIEW_LIST,
        PREVIEW_OPEN,
        PREVIEW_SCREENSHOT,
        PREVIEW_CONSOLE,
        HANDBOOK_CHECK,
        HANDBOOK_ANSWERS,
        HANDBOOK_VISUAL_CHECK,
        BROWSER_READ,
        BROWSER_ACT,
        BROWSER_CAPTURE,
        BROWSER_CAPTURE_SECRET,
        SECRET_LIST,
        SECRET_RUN,
        SECRET_RUN_STATUS,
        LIBRARIES_LIST,
        LIBRARY_SEARCH,
        IMAGE_GENERATE,
        DOCUMENT_READ,
        READ_ALOUD,
        PANEL_READ,
        PROPOSE_PROTECTED_CHANGE,
        PROTECTED_CHANGE_STATUS,
        USE_YOUR_CHROME,
        USE_HEADLESS_BROWSER,
        CHROME_OUTCOME,
        "chrome_status",
        "chrome_task",
        "chrome_execute",
    ];
}

/// One request line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The caller's credential (hex).
    pub credential: String,
    /// Tool name.
    pub tool: String,
    /// Tool arguments (an object).
    #[serde(default)]
    pub args: Value,
}

/// One answer line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    /// The tool's result.
    Ok(Value),
    /// A plain-text reason (refusals and failures).
    Error(String),
}

/// Reads one `\n`-terminated line of at most [`MAX_LINE`] bytes.
///
/// # Errors
/// A text reason when the stream ends before a newline, the line is too long, or it is not UTF-8.
pub fn read_line(reader: &mut impl Read) -> Result<String, String> {
    read_line_max(reader, MAX_LINE)
}

/// Reads one `\n`-terminated line of at most `max_line` bytes.
///
/// # Errors
/// A text reason when the stream ends before a newline, the line is too long, or it is not UTF-8.
pub fn read_line_max(reader: &mut impl Read, max_line: usize) -> Result<String, String> {
    let mut limited = BufReader::new(reader.take(u64::try_from(max_line + 1).unwrap_or(u64::MAX)));
    let mut buf = Vec::new();
    limited
        .read_until(b'\n', &mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    if buf.last() != Some(&b'\n') {
        return Err(if buf.len() > max_line {
            "line too long".to_owned()
        } else {
            "connection closed before a full line".to_owned()
        });
    }
    buf.pop();
    String::from_utf8(buf).map_err(|_| "line is not UTF-8".to_owned())
}

/// Sends one request to the core at `endpoint` and returns its result.
///
/// # Errors
/// The core's plain-text refusal, or a text reason when the core cannot be reached or answers badly (the
/// credential is never part of it).
pub fn call(
    endpoint: &str,
    credential: &str,
    tool: &str,
    args: Value,
    timeout: impl Into<Option<Duration>>,
) -> Result<Value, String> {
    let addr: SocketAddr = endpoint
        .parse()
        .map_err(|_| "the Vibe Work endpoint is not an address".to_owned())?;
    if !addr.ip().is_loopback() {
        return Err("the Vibe Work endpoint is not a loopback address".to_owned());
    }
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("Vibe Work is not reachable: {e}"))?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let request = Request {
        credential: credential.to_owned(),
        tool: tool.to_owned(),
        args,
    };
    let mut line = serde_json::to_vec(&request).map_err(|e| format!("encode failed: {e}"))?;
    line.push(b'\n');
    stream
        .write_all(&line)
        .map_err(|e| unconfirmed_process(tool, format!("send to Vibe Work failed: {e}")))?;
    let text = match timeout.into() {
        Some(timeout) => read_answer_until(&mut stream, MAX_ANSWER, timeout),
        None => read_line_max(&mut stream, MAX_ANSWER),
    }
    .map_err(|reason| unconfirmed_process(tool, reason))?;
    match serde_json::from_str::<Answer>(&text) {
        Ok(Answer::Ok(value)) => Ok(value),
        Ok(Answer::Error(reason)) => Err(reason),
        Err(_) => Err(unconfirmed_process(
            tool,
            "Vibe Work sent an unreadable answer".to_owned(),
        )),
    }
}

// A socket timeout is only one poll. In particular macOS can report EAGAIN while a native
// permission dialog is still open. Keep the partial answer and enforce one elapsed deadline.
fn read_answer_until(
    stream: &mut TcpStream,
    max_line: usize,
    timeout: Duration,
) -> Result<String, String> {
    let started = Instant::now();
    let mut line = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("read failed: timed out waiting for Vibe Work answer".to_owned());
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(200))))
            .map_err(|e| format!("read failed: {e}"))?;
        let size = chunk.len().min(max_line + 1 - line.len());
        match stream.read(&mut chunk[..size]) {
            Ok(0) => return Err("connection closed before a full line".to_owned()),
            Ok(n) => {
                line.extend_from_slice(&chunk[..n]);
                if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
                    if end + 1 > max_line {
                        return Err("line too long".to_owned());
                    }
                    return String::from_utf8(line[..end].to_vec())
                        .map_err(|_| "line is not UTF-8".to_owned());
                }
                if line.len() > max_line {
                    return Err("line too long".to_owned());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                // Some platforms return WouldBlock before their timer has elapsed.
                std::thread::sleep(Duration::from_millis(5).min(remaining));
            }
            Err(e) => return Err(format!("read failed: {e}")),
        }
    }
}

fn unconfirmed_process(tool: &str, reason: String) -> String {
    if matches!(tool, tools::PROCESS_START | tools::PROCESS_RESTART) {
        format!(
            "{UNKNOWN_OUTCOME} could not confirm {tool}: {reason}. The process may have started; call processes_list to check its pid and readiness before repeating the request."
        )
    } else if tools::CHROME_TOOLS.contains(&tool) {
        format!(
            "{UNKNOWN_OUTCOME} Chrome Control did not return a confirmed reply. Check Chrome before trying again. ({reason})"
        )
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lost_chrome_reply_is_unknown_and_must_not_be_repeated() {
        let text = unconfirmed_process("chrome_execute", "pipe closed".to_owned());
        assert!(text.starts_with(UNKNOWN_OUTCOME));
        assert!(text.contains("Check Chrome"));
    }

    #[test]
    fn answer_survives_idle_socket_polls_and_keeps_partial_json() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_line(&mut stream).unwrap();
            stream.write_all(b"{\"ok\":").unwrap();
            std::thread::sleep(Duration::from_millis(450));
            stream.write_all(b"{\"done\":true}}\n").unwrap();
        });
        let result = call(
            &endpoint,
            "synthetic-test",
            tools::BROWSER_ACT,
            json!({}),
            Duration::from_secs(2),
        );
        assert_eq!(result.unwrap(), json!({"done": true}));
        server.join().unwrap();
    }

    #[test]
    fn partial_answer_expires_at_one_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let (release, held) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_line(&mut stream).unwrap();
            stream.write_all(b"{\"ok\":").unwrap();
            held.recv().unwrap();
        });
        let started = Instant::now();
        let reason = call(
            &endpoint,
            "synthetic-test",
            tools::BROWSER_ACT,
            json!({}),
            Duration::from_millis(250),
        )
        .unwrap_err();
        assert_eq!(
            reason,
            "read failed: timed out waiting for Vibe Work answer"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        server.join().unwrap();
    }
}
