//! MCP server definitions in the four agents' formats, and which of their settings are keys.
//!
//! Formats: the `~/.claude/` agent and Cursor (`mcpServers`: `{command, args, env}` or `{type, url, headers}`), Codex
//! (`[mcp_servers.<name>]`: `command`, `args`, `env`, or `url`), OpenCode (`mcp`: `{type: "local", command: [...],
//! environment}` or `{type: "remote", url, headers}`).
//!
//! A key is any header or environment setting whose value is written out (not a `${VAR}` / `$VAR` / `{env:VAR}`
//! reference), whatever its name: a name cannot prove a value is not a credential, and Vibe's config would hold it as
//! plain text. In a command line or an address, a key is a setting whose name says so (`*KEY*`, `*TOKEN*`,
//! `*SECRET*`, `*PASSWORD*`, `Authorization`, `Cookie`). A key's value never leaves [`Server`]: `Debug` and the plan
//! show names only.
//!
//! Depends on: `serde_json`, `toml`. Called by: `agents`.

use serde_json::Value;

/// One server, ready to be added to Vibe Work's config.
pub enum Server {
    /// A program Vibe starts.
    Stdio {
        /// The program.
        command: String,
        /// Its arguments.
        args: Vec<String>,
        /// Its environment (names and values).
        env: Vec<(String, String)>,
    },
    /// A server at an address.
    Http {
        /// The address.
        url: String,
        /// Request headers (names and values).
        headers: Vec<(String, String)>,
    },
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio { command, args, env } => write!(
                f,
                "Stdio {{ command: {command:?}, args: {args:?}, env: {:?} }}",
                env.iter().map(|(k, _)| k).collect::<Vec<_>>()
            ),
            Self::Http { url, headers } => write!(
                f,
                "Http {{ url: {url:?}, headers: {:?} }}",
                headers.iter().map(|(k, _)| k).collect::<Vec<_>>()
            ),
        }
    }
}

/// A setting name that holds a key.
#[must_use]
pub fn key_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"]
        .iter()
        .any(|w| upper.contains(w))
        || matches!(
            upper.as_str(),
            "AUTHORIZATION" | "COOKIE" | "PROXY-AUTHORIZATION"
        )
}

/// A value that is written out (not a reference to an environment variable).
#[must_use]
pub fn literal(value: &str) -> bool {
    let v = value.trim();
    !(v.is_empty()
        || v.starts_with("${")
        || (v.starts_with('$') && !v.contains(' '))
        || v.starts_with("{env:")
        || v.starts_with("env:"))
}

impl Server {
    /// The names of its header or environment settings with a written-out value (each counts as a key).
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let pairs = match self {
            Self::Stdio { env, .. } => env,
            Self::Http { headers, .. } => headers,
        };
        pairs
            .iter()
            .filter(|(_, v)| literal(v))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Keys written into the command line (`--token=x`, `--api-key x`, `API_KEY=x`) or the address (a user name
    /// and password, a query parameter named like a key): named by where they are, never by value.
    #[must_use]
    pub fn inline_keys(&self) -> Vec<String> {
        match self {
            Self::Stdio { args, .. } => {
                let mut found = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    if let Some((name, value)) = arg.split_once('=')
                        && key_name(name.trim_start_matches('-'))
                        && literal(value)
                    {
                        found.push(name.to_owned());
                    } else if arg.starts_with('-')
                        && key_name(arg.trim_start_matches('-'))
                        && args
                            .get(i + 1)
                            .is_some_and(|v| literal(v) && !v.starts_with('-'))
                    {
                        found.push(arg.clone());
                    }
                }
                found
            }
            Self::Http { url, .. } => {
                let mut found = Vec::new();
                // A fragment is never needed by a server and can carry a token: it counts as a key.
                let (url, fragment) = url
                    .split_once('#')
                    .map_or((url.as_str(), None), |(u, f)| (u, Some(f)));
                if fragment.is_some() {
                    found.push("the address's fragment".to_owned());
                }
                let rest = url.split_once("://").map_or(url, |(_, r)| r);
                if rest
                    .split('/')
                    .next()
                    .is_some_and(|host| host.contains('@'))
                {
                    found.push("the address's user and password".to_owned());
                }
                if let Some((_, query)) = url.split_once('?') {
                    for pair in query.split('&') {
                        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                        if key_name(name) && literal(value) {
                            found.push(format!("the address's {name}"));
                        }
                    }
                }
                found
            }
        }
    }

    /// Environment names that change how any program runs (search paths, preloaded code, interpreter options).
    #[must_use]
    pub fn risky_env(&self) -> Vec<String> {
        let Self::Stdio { env, .. } = self else {
            return Vec::new();
        };
        env.iter()
            .map(|(k, _)| k)
            .filter(|k| {
                let upper = k.to_ascii_uppercase();
                matches!(
                    upper.as_str(),
                    "PATH"
                        | "NODE_OPTIONS"
                        | "NODE_PATH"
                        | "PYTHONPATH"
                        | "PYTHONSTARTUP"
                        | "PYTHONHOME"
                        | "PERL5LIB"
                        | "PERL5OPT"
                        | "RUBYOPT"
                        | "RUBYLIB"
                        | "JAVA_TOOL_OPTIONS"
                        | "BASH_ENV"
                        | "ENV"
                        | "COMSPEC"
                        | "PATHEXT"
                ) || upper.starts_with("LD_")
                    || upper.starts_with("DYLD_")
            })
            .cloned()
            .collect()
    }

    /// The command line (with its environment, `NAME=value`) or address shown before anything is brought. A key
    /// written inline and every written-out environment value show as `•••`; an address shows without its user and
    /// password, query and fragment. The text is safe to show to anyone who may read the plan.
    #[must_use]
    pub fn detail(&self) -> String {
        const HIDDEN: &str = "\u{2022}\u{2022}\u{2022}";
        let quote = |part: &str| {
            if part.contains(char::is_whitespace) || part.is_empty() {
                format!("{part:?}")
            } else {
                part.to_owned()
            }
        };
        match self {
            Self::Stdio { command, args, env } => {
                let mut parts: Vec<String> = env
                    .iter()
                    .map(|(k, v)| {
                        if literal(v) {
                            format!("{k}={HIDDEN}")
                        } else {
                            format!("{k}={}", quote(v))
                        }
                    })
                    .collect();
                parts.push(quote(command));
                let mut hide_next = false;
                for arg in args {
                    if hide_next {
                        parts.push(HIDDEN.to_owned());
                        hide_next = false;
                        continue;
                    }
                    if let Some((name, value)) = arg.split_once('=')
                        && key_name(name.trim_start_matches('-'))
                        && literal(value)
                    {
                        parts.push(format!("{name}={HIDDEN}"));
                        continue;
                    }
                    hide_next = arg.starts_with('-') && key_name(arg.trim_start_matches('-'));
                    parts.push(quote(arg));
                }
                parts.join(" ")
            }
            Self::Http { url, .. } => {
                // The address's parts in the order of its syntax (RFC 3986): the fragment, then the query, then the
                // authority up to the path. The user and password and the whole query and fragment are not shown.
                let (url, fragment) = url
                    .split_once('#')
                    .map_or((url.as_str(), None), |(u, f)| (u, Some(f)));
                let (url, query) = url
                    .split_once('?')
                    .map_or((url, None), |(u, q)| (u, Some(q)));
                let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
                let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
                let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
                let mut shown = format!("{scheme}://{host}/{path}");
                if query.is_some() {
                    shown.push('?');
                    shown.push_str(HIDDEN);
                }
                if fragment.is_some() {
                    shown.push('#');
                    shown.push_str(HIDDEN);
                }
                shown
            }
        }
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
        .collect()
}

/// The `~/.claude/` agent and Cursor: one entry of `mcpServers`.
#[must_use]
pub fn from_claude(entry: &Value) -> Option<Server> {
    if let Some(url) = entry.get("url").and_then(Value::as_str) {
        return Some(Server::Http {
            url: url.to_owned(),
            headers: pairs(entry.get("headers")),
        });
    }
    Some(Server::Stdio {
        command: entry.get("command")?.as_str()?.to_owned(),
        args: strings(entry.get("args")),
        env: pairs(entry.get("env")),
    })
}

/// OpenCode: one entry of `mcp`.
#[must_use]
pub fn from_opencode(entry: &Value) -> Option<Server> {
    if entry.get("type").and_then(Value::as_str) == Some("remote") {
        return Some(Server::Http {
            url: entry.get("url")?.as_str()?.to_owned(),
            headers: pairs(entry.get("headers")),
        });
    }
    let mut command = strings(entry.get("command")).into_iter();
    Some(Server::Stdio {
        command: command.next()?,
        args: command.collect(),
        env: pairs(entry.get("environment")),
    })
}

/// Codex: one `[mcp_servers.<name>]` table.
#[must_use]
pub fn from_codex(entry: &toml::Value) -> Option<Server> {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let table = |key: &str| -> Vec<(String, String)> {
        entry
            .get(key)
            .and_then(toml::Value::as_table)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
            .collect()
    };
    if let Some(url) = text("url") {
        return Some(Server::Http {
            url,
            headers: table("http_headers"),
        });
    }
    Some(Server::Stdio {
        command: text("command")?,
        args: entry
            .get("args")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        env: table("env"),
    })
}

/// JSON with `//` and `/* */` comments (OpenCode's `opencode.jsonc`), comments removed outside strings.
#[must_use]
pub fn jsonc(text: &str) -> Option<Value> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for skipped in chars.by_ref() {
                    if last == '*' && skipped == '/' {
                        break;
                    }
                    last = skipped;
                }
            }
            _ => out.push(c),
        }
    }
    serde_json::from_str(&out).ok()
}
