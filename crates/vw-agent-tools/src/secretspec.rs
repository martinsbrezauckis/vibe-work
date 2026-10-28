//! The request of the agent tool `secret_run`: one
//! program with arguments (no shell), a working folder, and the secrets that go into named environment variables.
//! Pure parsing and checks, in the shape of ShellX Vault's injection spec (`validate_injection_spec`,
//! `validate_env_name`, `reject_inline_interpreter_evaluation`), written anew.
//!
//! Why no shell and no inline evaluation: the user approves the exact program and arguments in the dialog; a
//! `sh -c '...'`, `python -c '...'` or `node -e '...'` would let a text the dialog shows as one argument decide
//! what happens to the value (print it reversed, send it away). A script file is allowed (its name is in the
//! dialog), like in Vault. This is a check against the common forms, not a sandbox: output is scrubbed anyway.
//!
//! Depends on: `serde_json`. Called by: vw-core-api (`agent_tools/secret_run`), tests.
//! Invariant: every function is pure (no I/O except [`resolve_program`], which only checks files exist).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Most secrets in one request.
pub const MAX_BINDINGS: usize = 8;
/// Most arguments.
pub const MAX_ARGS: usize = 64;
/// Longest argument (bytes).
pub const MAX_ARG: usize = 4096;
/// Longest purpose (characters).
pub const MAX_PURPOSE: usize = 300;
/// Default run time limit (seconds).
pub const DEFAULT_TIMEOUT_S: u64 = 120;
/// Longest run time limit (seconds).
pub const MAX_TIMEOUT_S: u64 = 900;

/// One secret into one variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The environment variable name.
    pub env: String,
    /// The secret's id or name (as `secret_list` shows it).
    pub secret: String,
    /// A secret field (default: the secret's value field).
    pub field: Option<String>,
}

/// A parsed `secret_run` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSpec {
    /// The program (a name found on `PATH`, or a path).
    pub program: String,
    /// Its arguments, each passed as one argument (no shell).
    pub args: Vec<String>,
    /// Working folder (relative to the session folder, or absolute inside it).
    pub cwd: Option<String>,
    /// The secrets.
    pub bindings: Vec<Binding>,
    /// Why, in the agent's words (kept in the tool call for compatibility).
    pub purpose: String,
    /// Run time limit, seconds.
    pub timeout_s: u64,
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn plain(value: &str) -> bool {
    !value.contains('\0') && !value.chars().any(hidden)
}

/// Characters that hide or reorder text in a dialog (bidirectional controls, zero-width marks).
fn hidden(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

/// Parses and checks the tool arguments.
///
/// # Errors
/// A plain-text reason for the agent.
pub fn parse(args: &Value) -> Result<RunSpec, String> {
    let program = text(args, "program")
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty() && p.len() <= MAX_ARG && plain(p))
        .ok_or(
            "program must be the program to run (a name on PATH or a path), without arguments",
        )?;
    if program.contains(char::is_whitespace) && !Path::new(&program).is_absolute() {
        return Err(
            "program is one program name; put its arguments in args (there is no shell)".to_owned(),
        );
    }
    let list = match args.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or("args must be a list of strings")?,
        Some(_) => return Err("args must be a list of strings".to_owned()),
    };
    if list.len() > MAX_ARGS || list.iter().any(|a| a.len() > MAX_ARG || !plain(a)) {
        return Err(format!(
            "args: at most {MAX_ARGS} arguments of at most {MAX_ARG} bytes, without NUL or hidden characters"
        ));
    }
    let purpose = text(args, "purpose")
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty() && p.chars().count() <= MAX_PURPOSE && plain(p))
        .ok_or("purpose must say in one sentence why the command needs the secret")?;
    let cwd = text(args, "cwd").filter(|c| !c.trim().is_empty());
    let timeout_s = match args.get("timeoutSeconds") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_S,
        Some(v) => v
            .as_u64()
            .filter(|s| (1..=MAX_TIMEOUT_S).contains(s))
            .ok_or(format!("timeoutSeconds must be 1 to {MAX_TIMEOUT_S}"))?,
    };
    let bindings = parse_bindings(args.get("env"))?;
    if let Some(reason) = inline_evaluation(&program, &list) {
        return Err(reason);
    }
    Ok(RunSpec {
        program,
        args: list,
        cwd,
        bindings,
        purpose,
        timeout_s,
    })
}

fn parse_bindings(value: Option<&Value>) -> Result<Vec<Binding>, String> {
    let items = value
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_BINDINGS)
        .ok_or(format!(
            "env must list 1 to {MAX_BINDINGS} entries {{\"name\": \"VARIABLE\", \"secret\": \"<id or name from secret_list>\"}}"
        ))?;
    let mut out: Vec<Binding> = Vec::new();
    for item in items {
        let env = text(item, "name").ok_or("each env entry needs a name")?;
        validate_env_name(&env)?;
        if out.iter().any(|b| b.env.eq_ignore_ascii_case(&env)) {
            return Err(format!("{env} is listed twice"));
        }
        let secret = text(item, "secret")
            .filter(|s| !s.trim().is_empty() && s.len() <= 256 && plain(s))
            .ok_or("each env entry needs the secret's id or name from secret_list")?;
        let field = text(item, "field").filter(|f| !f.trim().is_empty());
        out.push(Binding {
            env,
            secret: secret.trim().to_owned(),
            field,
        });
    }
    Ok(out)
}

/// Variables that change how programs load or where they look (ShellX Vault's list plus a few).
const DENIED_EXACT: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SYSTEMROOT",
    "WINDIR",
    "HOME",
    "USERPROFILE",
    "NODE_OPTIONS",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "BASH_ENV",
    "ENV",
    "RUSTC_WRAPPER",
    "GIT_SSH_COMMAND",
    "GIT_CONFIG_GLOBAL",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "PERL5OPT",
    "RUBYOPT",
    "JAVA_TOOL_OPTIONS",
    "PROMPT_COMMAND",
    "IFS",
    "SHELLOPTS",
];
/// Prefixes of such variables (and Vibe Work's own).
const DENIED_PREFIXES: &[&str] = &["LD_", "DYLD_", "MALLOC_", "ASAN_", "TSAN_", "VW_"];

/// Checks an environment variable name: letters, digits and `_`, not starting with a digit, at most 128 bytes, and
/// not one that changes how programs load or where they look (Vault's list).
///
/// # Errors
/// The reason.
pub fn validate_env_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let start = chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic());
    if !start || name.len() > 128 || !chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return Err(format!(
            "{} is not a valid environment variable name",
            name.chars().take(40).collect::<String>()
        ));
    }
    let upper = name.to_ascii_uppercase();
    if DENIED_EXACT.contains(&upper.as_str())
        || DENIED_PREFIXES.iter().any(|p| upper.starts_with(p))
    {
        return Err(format!(
            "{name} changes how programs load or run; use a variable the program reads its key from"
        ));
    }
    Ok(())
}

/// The program's file name in lower case, without a Windows `.exe` / `.cmd` / `.bat` ending.
fn base_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    for ending in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(stem) = name.strip_suffix(ending) {
            return stem.to_owned();
        }
    }
    name
}

/// `name` is `prefix` followed by nothing or a version (`python3`, `python3.12`, `perl5`).
fn versioned(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// True when one of `args` before the first non-option is a short option holding `letter`, or equals one of `long`.
fn has_option(args: &[String], letters: &str, long: &[&str]) -> bool {
    for arg in args {
        if arg == "--" || !arg.starts_with('-') {
            return false;
        }
        let lower = arg.to_ascii_lowercase();
        if long
            .iter()
            .any(|l| lower == *l || lower.starts_with(&format!("{l}=")))
        {
            return true;
        }
        if !arg.starts_with("--") && arg[1..].chars().any(|c| letters.contains(c)) {
            return true;
        }
    }
    false
}

/// The wrapped program and its arguments of a wrapper command (`env A=1 node x`, `timeout 5 node x`).
fn wrapped<'a>(wrapper: &str, args: &'a [String]) -> Option<(&'a str, &'a [String])> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if wrapper == "env" && arg.contains('=') && !arg.starts_with('-') {
            index += 1;
            continue;
        }
        if !arg.starts_with('-') {
            break;
        }
        let takes_value = matches!(
            (wrapper, arg.as_str()),
            ("env", "-u" | "--unset" | "-C" | "--chdir")
                | ("nice", "-n" | "--adjustment")
                | ("timeout", "-s" | "--signal" | "-k" | "--kill-after")
                | ("stdbuf", "-i" | "-o" | "-e")
        );
        index += if takes_value { 2 } else { 1 };
    }
    if wrapper == "timeout" {
        index += 1; // the duration
    }
    let program = args.get(index)?;
    Some((program.as_str(), args.get(index + 1..).unwrap_or(&[])))
}

/// The refusal when `program args` evaluates code given inline instead of running a program or a script file,
/// else `None`.
#[must_use]
pub fn inline_evaluation(program: &str, args: &[String]) -> Option<String> {
    let name = base_name(program);
    let refused = match name.as_str() {
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ash" | "ksh" | "csh" | "tcsh" => {
            has_option(args, "c", &["--command", "--rcfile", "--init-file"])
        }
        "cmd" => args.iter().take_while(|a| a.starts_with('/')).any(|a| {
            let lower = a.to_ascii_lowercase();
            lower.starts_with("/c") || lower.starts_with("/k")
        }),
        "powershell" | "pwsh" => {
            args.iter().any(|a| {
                let lower = a.to_ascii_lowercase();
                let Some(param) = lower.strip_prefix(['-', '/']) else {
                    return false;
                };
                // PowerShell takes any unambiguous prefix of a parameter name (`-c`, `-com`, `-enc`).
                !param.is_empty()
                    && (["command", "encodedcommand", "commandwithargs"]
                        .iter()
                        .any(|full| full.starts_with(param))
                        || ["ec", "cwa"].contains(&param))
            }) || args
                .first()
                .is_some_and(|a| !a.starts_with('-') && !a.to_ascii_lowercase().ends_with(".ps1"))
        }
        "node" | "nodejs" | "bun" => {
            has_option(args, "ep", &["--eval", "--print", "--require", "--import"])
        }
        "deno" => args.first().is_some_and(|a| a == "eval") || has_option(args, "", &["--eval"]),
        "awk" | "gawk" | "mawk" | "nawk" | "sed" | "jq" => true,
        "osascript" | "rscript" | "r" => has_option(args, "e", &["--expr"]),
        "php" => has_option(args, "r", &["--run"]),
        "find" => args
            .iter()
            .any(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")),
        "env" if has_option(args, "S", &["--split-string"]) => true,
        "env" | "timeout" | "nohup" | "nice" | "setsid" | "stdbuf" | "xargs" | "sudo" | "doas"
        | "busybox" | "wsl" | "command" | "exec" | "time" => {
            return match wrapped(&name, args) {
                Some((inner, rest)) => inline_evaluation(inner, rest),
                // A bare `env` prints the environment: the output scrub hides the values, but there is no reason
                // to run it.
                None => Some(format!(
                    "{name} without a program to run is not useful here"
                )),
            };
        }
        n if versioned(n, "python") || versioned(n, "pypy") || n == "py" || n == "ipython" => {
            has_option(args, "c", &[])
        }
        n if versioned(n, "ruby") || versioned(n, "perl") => has_option(args, "eE", &["--eval"]),
        _ => false,
    };
    refused.then(|| {
        format!(
            "{name} with code given inline is not run with a secret: the user approves the program and its \
             arguments, so write the code into a script file in the project and run that file instead"
        )
    })
}

/// Finds `program`: a path (absolute, or with a separator: relative to `cwd`) must be a file; a bare name is looked
/// up on `path` (with `pathext` endings on Windows). `None` when not found.
#[must_use]
pub fn resolve_program(
    program: &str,
    cwd: &Path,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() || program.contains(['/', '\\']) {
        let full = cwd.join(candidate);
        return full.is_file().then_some(full);
    }
    let endings: Vec<String> = if cfg!(windows) {
        let mut list = vec![String::new()];
        list.extend(
            pathext
                .and_then(OsStr::to_str)
                .unwrap_or(".COM;.EXE;.BAT;.CMD")
                .split(';')
                .filter(|e| !e.is_empty())
                .map(str::to_ascii_lowercase),
        );
        list
    } else {
        vec![String::new()]
    };
    std::env::split_paths(path?).find_map(|dir| {
        endings.iter().find_map(|ending| {
            let full = dir.join(format!("{program}{ending}"));
            full.is_file().then_some(full)
        })
    })
}
