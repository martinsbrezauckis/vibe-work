//! The scripted model: a stand-in on 127.0.0.1 where Mistral's API would be, so the REAL Vibe runs real
//! turns that stay deterministic, free and key-free. Vibe itself is never replaced: only the model's answers are.
//!
//! Wire, as the pinned Vibe 3.1.2 speaks it (the versioned Python 3.14 environment):
//! - Vibe's `mistral` backend streams every agent turn with `chat.stream_async` (`vibe/core/llm/backend/mistral.py:570`)
//!   and makes side calls (titles, summaries) with `chat.complete_async` (`mistral.py:488`);
//! - the SDK (mistralai 3.5.0) posts both to `<server>/v1/chat/completions` (`mistralai/client/chat.py:236` and `:624`,
//!   the streamed one as server-sent events `data: <CompletionChunk>` ended by `data: [DONE]`);
//! - `<server>` is the provider's `api_base` without its `/v<N>` suffix (`vibe/utils/http.py:223`), so the provider
//!   gets `api_base = <base>/v1`; usage comes from each chunk's `usage` (`mistral.py:604`).
//!
//! The fixture checks the Bearer header and provider model id before replying. Real-server rigs keep the product
//! key proxy in between.
//!
//! Any other path is answered 404 and logged, so a test sees when Vibe calls something new; with a `fallback` base
//! (`start_with_fallback`) it is passed on over HTTP instead (the e2e host's fake Mistral for Vibe Work's own calls:
//! the key check, OCR, speech; Vibe's model goes through Vibe Work's key proxy there, whose upstream is this model).
//!
//! What a reply can be (`Reply`): text (in `chunks` pieces, `chunk_ms` apart), tool calls (any of Vibe's tools, or
//! `edit_tokens`: one `write_file` per `edit:<path>` token of the user message), a delay before answering, an HTTP
//! error, a malformed tool call (arguments that are not JSON), and the usage numbers. Which reply: the first matching
//! `Rule` of the script (`user` regex on the last user message, `step` = tool rounds since it, `tool` regex on the
//! last tool result, `side` for non-agent calls), else a Rust hook, else the script's `default`, else "ok".
//!
//! Depends on: axum, regex, serde_json, tokio. Called by: `common::real_vibe` (the real-Vibe backend of the core
//! tests), `shell_host` (the e2e backend, `#[path]`), and tests that script a model directly.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "scripted_model_http.rs"]
mod http;
use http::handle;
use serde::Deserialize;
use serde_json::{Value, json};

/// One scripted step (see the module docs). Unknown fields are refused, so a typo in a script fails loudly.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Reply {
    /// Assistant text; `{user}` is replaced by the last user message.
    pub text: Option<String>,
    /// A Mistral thinking content block (tests of the transcript's reasoning controls).
    pub reasoning: Option<String>,
    /// Tool calls, in order (one assistant message).
    pub tools: Vec<Call>,
    /// One `write_file` call per `edit:<path>` token of the last user message (appends a line, like the old fixture).
    pub edit_tokens: bool,
    /// Wait this long before answering at all (a slow model).
    pub delay_ms: u64,
    /// Hold this model request until a disposable control file exists. JSON Reply content replaces the reply.
    pub wait_file: Option<String>,
    /// Split the text into this many chunks (default 1) ...
    pub chunks: usize,
    /// ... sent this far apart (a long answer that can be stopped mid-way).
    pub chunk_ms: u64,
    /// Answer this HTTP status with `error` as Mistral's error body instead.
    pub status: Option<u16>,
    pub error: Option<String>,
    /// The first tool call's arguments are not JSON.
    pub malformed: bool,
    /// Usage numbers of the answer (default 1 / 1).
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
}

/// When a reply applies. Every given condition must hold.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Rule {
    /// Regex on the last user message; its groups fill `${1}`, `${2}`, ... in the reply's text and tool arguments.
    pub user: Option<String>,
    /// Regex on the system prompt (e.g. Vibe's turn-summary prompt for narration).
    pub system: Option<String>,
    /// Tool rounds (assistant tool-call messages) since the last user message.
    pub step: Option<usize>,
    /// Regex on the last tool result (the request's last message must be a tool result).
    pub tool: Option<String>,
    /// `true`: only side requests (not streamed, or no tools offered: titles, summaries); `false`: only agent turns.
    pub side: Option<bool>,
    /// Use this rule at most this many times.
    pub times: Option<usize>,
    pub reply: Reply,
}

/// A whole script (JSON: `{"rules": [...], "default": {...}}`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Script {
    /// Opt-in isolated native tests may retain the model input (including image parts) as evidence.
    pub record_inputs: bool,
    pub rules: Vec<Rule>,
    pub default: Option<Reply>,
    /// Obey the e2e scripts' tool markers in the user's message (see [`marker`]) before the rules.
    pub markers: bool,
    /// Optional scripted Mistral registry; real Vibe still implements every skills/* command.
    pub registry: Option<super::scripted_registry::Registry>,
    pub platform: bool,
    /// The platform's catalog also lists DeepWiki, a connector that needs no sign-in (`no_auth_connect`).
    pub no_sign_in: bool,
    /// Mistral's model catalog answer (`GET /v1/model-catalog`, Vibe's `ModelCatalogResponse`), when set.
    pub model_catalog: Option<Value>,
}

impl Script {
    /// Parses a script file (panics with the file name on a bad script: a test bug).
    pub fn from_file(path: &std::path::Path) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("scripted model: cannot read {}: {e}", path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("scripted model: bad script {}: {e}", path.display()))
    }
}

/// One request as the model saw it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub query: Option<String>,
    /// Digest of the received Authorization header; never stores or logs credential bytes.
    pub authorization_sha256: String,
    pub body: Value,
    pub stream: bool,
    /// Tool names offered (empty for side calls).
    pub tools: Vec<String>,
    pub system: String,
    pub users: Vec<String>,
    /// Tool rounds since the last user message.
    pub step: usize,
    pub last_tool: Option<String>,
    /// The chat's folder, from Vibe's system prompt (`Absolute path: <cwd>`).
    pub cwd: Option<String>,
}

impl Seen {
    pub fn last_user(&self) -> &str {
        self.users.last().map_or("", String::as_str)
    }
    pub fn side(&self) -> bool {
        !self.stream || self.tools.is_empty()
    }
}

/// The text of a message's `content` (a string, or text parts).
pub fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// The person's own words of a user message: Vibe Work attaches its per-turn session facts as a
/// `vw://app/context` resource, which Vibe renders after them. Rules and markers match the words, as a model would
/// read the instruction; the whole request body stays available to tests that check the facts themselves.
fn own_words(text: &str) -> String {
    text.split_once("\n\nResource: vw://app/context\n")
        .map_or(text, |(words, _)| words)
        .to_owned()
}

fn seen(path: &str, body: &Value) -> Seen {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let last_user_at = messages.iter().rposition(|m| m["role"] == "user");
    let step = last_user_at.map_or(0, |at| {
        messages[at + 1..]
            .iter()
            .filter(|m| m["role"] == "assistant" && m["tool_calls"].is_array())
            .count()
    });
    let system = messages
        .iter()
        .filter(|m| m["role"] == "system")
        .map(|m| text_of(&m["content"]))
        .collect::<Vec<_>>()
        .join("\n");
    Seen {
        cwd: system
            .lines()
            .find_map(|l| l.trim().strip_prefix("Absolute path: ").map(str::to_owned)),
        path: path.to_owned(),
        query: None,
        authorization_sha256: String::new(),
        stream: body["stream"] == true,
        tools: body["tools"]
            .as_array()
            .map(|t| {
                t.iter()
                    .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        system,
        users: messages
            .iter()
            .filter(|m| m["role"] == "user")
            .map(|m| own_words(&text_of(&m["content"])))
            .collect(),
        step,
        last_tool: messages
            .last()
            .filter(|m| m["role"] == "tool")
            .map(|m| text_of(&m["content"])),
        body: body.clone(),
    }
}

/// A Rust hook: `Some(reply)` overrides the script for this request.
pub type Hook = Arc<dyn Fn(&Seen) -> Option<Reply> + Send + Sync>;

#[path = "model_proxy.rs"]
mod model_proxy;

struct Inner {
    expected_key: Option<String>,
    proxies: Vec<vw_keyproxy::ProxyHandle>,
    fallback: Option<String>,
    script: Script,
    used: Vec<usize>,
    hook: Option<Hook>,
    log: Vec<Seen>,
    calls: u64,
    active_replies: usize,
    platform: super::scripted_platform::Platform,
}

/// A running scripted model. Dropping it does not stop the server (it lives as long as the runtime).
#[derive(Clone)]
pub struct ScriptedModel {
    base: String,
    inner: Arc<Mutex<Inner>>,
}

impl ScriptedModel {
    /// Starts the model on a free loopback port.
    pub async fn start(script: Script) -> Self {
        Self::start_with_fallback(script, None).await
    }

    /// [`Self::start`]; paths other than the chat completions go to `fallback` (`http://127.0.0.1:<port>`).
    pub async fn start_with_fallback(script: Script, fallback: Option<String>) -> Self {
        let platform = super::scripted_platform::Platform::new(script.no_sign_in);
        let inner = Arc::new(Mutex::new(Inner {
            expected_key: None,
            proxies: Vec::new(),
            fallback,
            used: vec![0; script.rules.len()],
            script,
            hook: None,
            log: Vec::new(),
            calls: 0,
            active_replies: 0,
            platform,
        }));
        let app = axum::Router::new()
            .fallback(handle)
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("scripted model: bind 127.0.0.1");
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        inner.lock().unwrap().platform.base = base.trim_end_matches("/v1").to_owned();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { base, inner }
    }

    pub fn platform_enabled(&self) -> bool {
        self.inner.lock().unwrap().script.platform
    }

    /// How many no-sign-in connects reached the scripted platform.
    pub fn platform_connects(&self) -> usize {
        self.inner.lock().unwrap().platform.connects
    }

    /// A manually supplied KeyService may own a different fixture key than the seeding proxy.
    pub fn expect_key(&self, key: Option<&str>) {
        self.inner.lock().unwrap().expected_key = key.map(str::to_owned);
    }

    pub fn registry(&self) -> Option<super::scripted_registry::Registry> {
        self.inner.lock().unwrap().script.registry.clone()
    }

    /// The provider `api_base` (`http://127.0.0.1:<port>/v1`).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Replaces the script (rule use counts restart).
    pub fn set_script(&self, script: Script) {
        let mut inner = self.inner.lock().unwrap();
        inner.used = vec![0; script.rules.len()];
        inner.script = script;
    }

    /// Sets where non-chat paths go (see [`Self::start_with_fallback`]).
    pub fn set_fallback(&self, base: Option<String>) {
        self.inner.lock().unwrap().fallback = base;
    }

    /// The server URL without `/v1` (what Vibe Work's key proxy takes as its API upstream).
    pub fn server(&self) -> &str {
        self.base.trim_end_matches("/v1")
    }

    pub fn set_hook(&self, hook: Hook) {
        self.inner.lock().unwrap().hook = Some(hook);
    }

    pub fn clear_hook(&self) {
        self.inner.lock().unwrap().hook = None;
    }

    /// Main-model responses still held, streaming, or waiting to be dropped after cancellation.
    pub fn active_replies(&self) -> usize {
        self.inner.lock().unwrap().active_replies
    }

    /// Every request so far (agent turns and side calls, and unknown paths).
    pub fn seen(&self) -> Vec<Seen> {
        self.inner.lock().unwrap().log.clone()
    }

    /// Appends every request, as it arrives, to `path` as JSON lines (path, stream, offered tools, last user text,
    /// step, last tool result, the chat's folder, usage asked for is not known here): a run's record of what Vibe
    /// asked its model. Runs as a task on the current runtime.
    pub fn log_to(&self, path: std::path::PathBuf) {
        let model = self.clone();
        let record_inputs = self.inner.lock().unwrap().script.record_inputs;
        tokio::spawn(async move {
            let mut written = 0;
            loop {
                let seen = model.seen();
                if seen.len() > written
                    && let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                {
                    use std::io::Write as _;
                    for s in &seen[written..] {
                        let line = json!({"input": record_inputs.then_some(&s.body), "path": s.path, "query": s.query, "stream": s.stream, "tools": s.tools,
                            "user": s.last_user(), "step": s.step, "lastTool": s.last_tool, "cwd": s.cwd,
                            "authorizationSha256": s.authorization_sha256,
                            "model": s.body["model"], "reasoning": s.body["reasoning_effort"],
                            "toolCalls": s.body["messages"].as_array().into_iter().flatten().flat_map(|message|message["tool_calls"].as_array().into_iter().flatten()).cloned().collect::<Vec<_>>()});
                        let _ = writeln!(file, "{line}");
                    }
                    written = seen.len();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }

    /// The `config.toml` provider block that points Vibe's `mistral` provider here.
    pub fn provider_toml(&self) -> String {
        format!(
            "\n[[providers]]\nname = \"mistral\"\napi_base = \"{}\"\napi_key_env_var = \"MISTRAL_API_KEY\"\n\
             backend = \"mistral\"\n",
            self.base
        )
    }
}

fn pick(inner: &mut Inner, s: &Seen) -> Reply {
    if let Some(hook) = inner.hook.clone()
        && let Some(reply) = hook(s)
    {
        return reply;
    }
    if inner.script.markers
        && !s.side()
        && let Some(reply) = marker(s)
    {
        return reply;
    }
    for (i, rule) in inner.script.rules.iter().enumerate() {
        let rx = |p: &String| {
            regex::Regex::new(p).unwrap_or_else(|e| panic!("scripted model: bad regex {p:?}: {e}"))
        };
        let matches = |pattern: &Option<String>, text: Option<&str>| {
            pattern
                .as_ref()
                .is_none_or(|p| text.is_some_and(|t| rx(p).is_match(t)))
        };
        if rule.times.is_some_and(|n| inner.used[i] >= n)
            || rule.side.is_some_and(|side| side != s.side())
            || rule.step.is_some_and(|step| step != s.step)
            || !matches(&rule.user, Some(s.last_user()))
            || !matches(&rule.system, Some(&s.system))
            || !matches(&rule.tool, s.last_tool.as_deref())
        {
            continue;
        }
        inner.used[i] += 1;
        let mut reply = rule.reply.clone();
        if let Some(pattern) = &rule.user
            && let Some(caps) = rx(pattern).captures(s.last_user())
        {
            let fill = |text: &mut String| {
                for n in 1..caps.len() {
                    let group = caps.get(n).map_or("", |m| m.as_str());
                    *text = text.replace(&format!("${{{n}}}"), group);
                }
            };
            if let Some(text) = reply.text.as_mut() {
                fill(text);
            }
            if let Some(path) = reply.wait_file.as_mut() {
                fill(path);
            }
            for call in &mut reply.tools {
                fill_strings(&mut call.args, &fill);
            }
        }
        return reply;
    }
    inner.script.default.clone().unwrap_or_else(|| Reply {
        text: Some("ok".to_owned()),
        ..Reply::default()
    })
}

/// Applies `fill` to every string inside `value` (a tool call's arguments).
fn fill_strings(value: &mut Value, fill: &impl Fn(&mut String)) {
    match value {
        Value::String(text) => fill(text),
        Value::Array(items) => items.iter_mut().for_each(|v| fill_strings(v, fill)),
        Value::Object(map) => map.values_mut().for_each(|v| fill_strings(v, fill)),
        _ => {}
    }
}

/// The e2e scripts' tool markers, which the old fake played itself; here the MODEL obeys them, so the real Vibe runs
/// the call (its own card, permission scope, hooks and tool):
/// - `vw-card: <tool> <json args>`: that tool (`file_system.` dropped: Vibe's model-side names are `bash`,
///   `write_file`, ...), the args without the old fake's `__` keys (Vibe derives the permission itself);
/// - `vw-bash: <command>`: `bash` with that command;
/// - `vw-mcp-card: <group>.<tool> <json args>`: the MCP tool through `run_typescript` (Vibe 2.26.1's code mode:
///   `tools.<group>.<tool>(args)`).
///
/// After the tool's result (step 1) the model answers `<marker> done: <the result, 300 characters>`.
fn marker(s: &Seen) -> Option<Reply> {
    let user = s.last_user().trim();
    let (kind, rest) = ["vw-card:", "vw-bash:", "vw-mcp-card:"]
        .iter()
        .find_map(|k| user.strip_prefix(k).map(|rest| (*k, rest.trim())))?;
    if s.step > 0 {
        let result: String = s
            .last_tool
            .clone()
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        return Some(Reply {
            text: Some(format!("{kind} done: {result}")),
            ..Reply::default()
        });
    }
    let call = match kind {
        "vw-bash:" => Call {
            name: "bash".to_owned(),
            args: json!({"command": rest}),
        },
        _ => {
            let (tool, args) = rest.split_once(' ').unwrap_or((rest, "{}"));
            let mut args: Value = serde_json::from_str(args.trim()).unwrap_or_else(|_| json!({}));
            if let Some(map) = args.as_object_mut() {
                map.retain(|k, _| !k.starts_with("__"));
            }
            if kind == "vw-mcp-card:" {
                let (group, leaf) = tool.rsplit_once('.').unwrap_or(("mcp", tool));
                Call {
                    name: "run_typescript".to_owned(),
                    args: json!({"code": format!(
                        "async function main() {{ return await tools.{group}.{leaf}({args}); }}"
                    )}),
                }
            } else {
                Call {
                    name: tool.strip_prefix("file_system.").unwrap_or(tool).to_owned(),
                    args,
                }
            }
        }
    };
    Some(Reply {
        tools: vec![call],
        ..Reply::default()
    })
}

/// `write_file` calls for the `edit:<path>` tokens of the last user message: each appends one line `edited in
/// <turn>` to the file in the chat's folder (created when missing), like the old fixture's `--edit-files`. A real
/// model would read the file first; the stand-in reads it from disk (it runs on the same machine as the sandbox) so
/// one tool round is enough. Tokens with `..` or an absolute path are skipped (the fixture's rule).
fn edit_calls(s: &Seen, turn: u64) -> Vec<Call> {
    s.last_user()
        .split_whitespace()
        .filter_map(|word| word.strip_prefix("edit:"))
        .filter(|rel| !rel.is_empty() && !rel.contains("..") && !rel.starts_with('/'))
        .map(|rel| {
            let previous = s
                .cwd
                .as_ref()
                .and_then(|cwd| std::fs::read_to_string(std::path::Path::new(cwd).join(rel)).ok())
                .unwrap_or_default();
            Call {
                name: "write_file".to_owned(),
                args: json!({"path": rel, "content": format!("{previous}edited in scripted-turn-{turn}\n")}),
            }
        })
        .collect()
}
