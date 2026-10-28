//! OS and filesystem faults around the pinned real Vibe process; never implements a protocol reply.
//! Test launchers supply a JSON fault plan and control file. Every protocol byte is forwarded unchanged.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn write(path: &Path, value: &Value) {
    std::fs::write(path, value.to_string()).unwrap();
}
fn read(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}
#[allow(dead_code)] // `tree` and `suspend` are used on Windows only.
#[path = "../../tests/common/launchers.rs"]
mod launchers;

/// Stops or continues the server. On Windows the whole tree below the started process: there the real Vibe is an
/// interpreter under its launchers (`launchers.rs`), and suspending only the launcher left Vibe running.
fn suspend(pid: u32, stop: bool) {
    #[cfg(unix)]
    assert!(
        Command::new("kill")
            .args([if stop { "-STOP" } else { "-CONT" }, &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    #[cfg(windows)]
    for id in launchers::tree(pid) {
        assert!(
            launchers::suspend(id, stop),
            "could not {} process {id}",
            if stop { "suspend" } else { "resume" }
        );
    }
}

/// Continues a suspended server after `suspendMs`, or once the file `suspendUntil` exists (a test that races a
/// second process against the suspended one says when that process is ready, however slow the machine is; the
/// server continues after 120 s at the latest, so a test that never says so cannot leave it stopped).
fn resume_later(pid: u32, rule: &Value) {
    let ms = rule["suspendMs"].as_u64();
    let until = rule["suspendUntil"].as_str().map(PathBuf::from);
    assert!(
        ms.is_some() || until.is_some(),
        "a suspension needs suspendMs or suspendUntil"
    );
    std::thread::spawn(move || {
        match (ms, until) {
            (Some(ms), _) => std::thread::sleep(Duration::from_millis(ms)),
            (None, Some(file)) => {
                let deadline = std::time::Instant::now() + Duration::from_secs(120);
                while !file.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            (None, None) => unreachable!(),
        }
        suspend(pid, false);
    });
}

/// A filesystem fault that makes Vibe's next storage or config write fail; undone when the reply passes.
///
/// `config-directory` / `env-directory` put a directory where Vibe expects its config or `.env` file.
/// `storage-link` makes the session store unusable for writes and deletes:
/// - Unix: the `unified` store becomes a symlink, which Vibe's storage refuses ("stored path cannot be a
///   symbolic link").
/// - Windows: Vibe's check (`Path.is_symlink()`) does not see a junction, and a real symlink needs a
///   privilege a normal user lacks, so the Windows failure a user really meets is used instead: a file
///   in the session folder held open by another program (an antivirus scan, a backup, the indexer).
///   The session's top-level files are held with read-only sharing, so Vibe can still read them but
///   replacing or deleting them fails (see `hold_session_files`).
enum ChangedPath {
    Moved {
        path: PathBuf,
        saved: PathBuf,
    },
    #[cfg(windows)]
    Held {
        files: Vec<std::fs::File>,
        created: Option<PathBuf>,
    },
}
impl ChangedPath {
    fn apply(kind: &str, message: &Value) -> Self {
        let home = PathBuf::from(std::env::var_os("VIBE_HOME").unwrap());
        // Joined one component at a time: a `/` left inside a Windows path is refused by `mklink`
        // ("Parameter format not correct").
        let path = match kind {
            "config-directory" => home.join("config.toml"),
            "env-directory" => home.join(".env"),
            _ => home.join("logs").join("session").join("unified"),
        };
        #[cfg(windows)]
        if !matches!(kind, "config-directory" | "env-directory") {
            return Self::hold_session_files(&path, message);
        }
        #[cfg(not(windows))]
        let _ = message;
        let saved = path.with_extension("fault-original");
        if !path.exists() {
            std::fs::create_dir_all(&path).unwrap();
        }
        std::fs::rename(&path, &saved).unwrap();
        if matches!(kind, "config-directory" | "env-directory") {
            std::fs::create_dir(&path).unwrap();
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&saved, &path).unwrap();
        }
        Self::Moved { path, saved }
    }

    /// Holds the session's files open with read-only sharing (no `FILE_SHARE_WRITE` /
    /// `FILE_SHARE_DELETE`), the way another program's open handle blocks Vibe:
    /// - the lease diagnostic `active/<id>.lock.json`, which Vibe rewrites when it takes the session's
    ///   lease (a delete of a chat that is not open takes it first); created if missing;
    /// - every file directly in the stored folder `unified/<id>` (an update of an open chat replaces
    ///   `CURRENT` there), when that folder exists.
    #[cfg(windows)]
    fn hold_session_files(unified: &Path, message: &Value) -> Self {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let hold = |path: &Path| {
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(path)
                .unwrap_or_else(|e| panic!("hold {}: {e}", path.display()))
        };
        let sid = message["params"]["sessionId"]
            .as_str()
            .expect("a storage-link fault on Windows needs a request that names its session");
        let active = unified.with_file_name("active");
        std::fs::create_dir_all(&active).unwrap();
        let diagnostic = active.join(format!("{sid}.lock.json"));
        let created = (!diagnostic.exists()).then(|| {
            std::fs::write(&diagnostic, b"").unwrap();
            diagnostic.clone()
        });
        let mut files = vec![hold(&diagnostic)];
        if let Ok(entries) = std::fs::read_dir(unified.join(sid)) {
            files.extend(
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
                    .map(|entry| hold(&entry.path())),
            );
        }
        Self::Held { files, created }
    }
}
impl Drop for ChangedPath {
    fn drop(&mut self) {
        match self {
            Self::Moved { path, saved } => {
                let _ = std::fs::remove_file(&*path);
                if path.exists() {
                    let _ = std::fs::remove_dir(&*path);
                }
                std::fs::rename(&*saved, &*path).unwrap();
            }
            #[cfg(windows)]
            Self::Held { files, created } => {
                files.clear();
                if let Some(file) = created {
                    let _ = std::fs::remove_file(&*file);
                }
            }
        }
    }
}
fn block_read(sid: &str, delay: u64, plan: &Path) -> std::process::Child {
    let home = PathBuf::from(std::env::var_os("VIBE_HOME").unwrap());
    let root = std::env::var_os("VIBE_SESSION_LOGGING__SAVE_DIR")
        .map_or_else(|| home.join("logs").join("session"), PathBuf::from);
    let store = root.join("unified").join(sid);
    let current = read(&store.join("CURRENT"));
    let manifest = store
        .join("generations")
        .join(current["generation"].as_str().unwrap())
        .join("manifest.json");
    let script = plan.with_extension("read-fault.py");
    std::fs::write(&script, include_str!("storage_read_fault.py")).unwrap();
    let python = std::env::var_os("VW_FAULT_PYTHON")
        .map(PathBuf::from)
        .expect("fault plan supplies the pinned Python");
    let mut child = Command::new(python)
        .arg(script)
        .arg(manifest)
        .arg(delay.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "READY", "storage fault did not become ready");
    child
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--sleeper") {
        std::thread::sleep(Duration::from_secs(600));
        return;
    }
    let split = args
        .iter()
        .position(|a| a == "--")
        .expect("--plan FILE -- SERVER ARGS");
    assert_eq!(args[0], "--plan");
    let plan_path = PathBuf::from(&args[1]);
    let mut plan = read(&plan_path);
    let state = plan_path.with_extension("starts");
    let starts = read(&state).as_u64().unwrap_or(0) + 1;
    write(&state, &json!(starts));
    if let Some(file) = plan["childLaunchFile"].as_str() {
        let _ = std::fs::remove_file(file);
    }
    let server = &args[split + 1..];
    let mut child = Command::new(&server[0])
        .args(&server[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let pid = if let Some(file) = plan["childLaunchFile"].as_str() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(pid) = read(Path::new(file))["pid"].as_u64() {
                break u32::try_from(pid).unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recording tap did not publish its child PID"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    } else {
        child.id()
    };
    if starts >= plan["crashFromStart"].as_u64().unwrap_or(u64::MAX)
        && starts <= plan["crashUntilStart"].as_u64().unwrap_or(u64::MAX)
    {
        let _ = child.kill();
        let _ = child.wait();
        std::process::exit(7);
    }

    if starts >= plan["suspendFromStart"].as_u64().unwrap_or(u64::MAX) {
        suspend(pid, true);
        resume_later(pid, &plan);
    }

    let spawn = |separate: bool| {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.arg("--sleeper")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            if separate {
                cmd.process_group(0);
            }
        }
        #[cfg(not(unix))]
        let _ = separate;
        cmd.spawn().unwrap()
    };
    let mut grandchild = plan["spawnGrandchild"]
        .as_bool()
        .unwrap_or(false)
        .then(|| spawn(false));
    let mut agent = plan["spawnAgent"]
        .as_bool()
        .unwrap_or(false)
        .then(|| spawn(true));
    let info = json!({"pid":std::process::id(),"server":pid,"exe":server[0].to_string_lossy(),"argv":server.iter().map(|arg|arg.to_string_lossy()).collect::<Vec<_>>(),"cwd":std::env::current_dir().unwrap(),"grandchild":grandchild.as_ref().map(std::process::Child::id),"agent":agent.as_ref().map(std::process::Child::id),"env":std::env::vars().collect::<std::collections::BTreeMap<_,_>>()});
    write(&plan_path.with_extension("info"), &info);
    if plan["stderrKey"].as_bool() == Some(true) {
        eprintln!(
            "Bearer {}\nTraceback\nFAULT-TRACEBACK-TEXT",
            std::env::var("MISTRAL_API_KEY").unwrap_or_default()
        );
    }
    let exit_code = Arc::new(std::sync::atomic::AtomicI32::new(i32::MIN));
    let control_exit = exit_code.clone();
    let control = plan_path.with_extension("control");
    std::thread::spawn(move || {
        loop {
            let action = read(&control);
            if let Some(code) = action["exit"].as_i64() {
                std::fs::remove_file(&control).unwrap();
                control_exit.store(
                    i32::try_from(code).unwrap(),
                    std::sync::atomic::Ordering::SeqCst,
                );
                eprint!("{}", action["stderr"].as_str().unwrap_or(""));
                // Windows: the whole tree (`/T`), so the interpreter behind a launcher dies too.
                let _ = Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
                    .args(if cfg!(windows) {
                        vec![
                            "/F".to_owned(),
                            "/T".to_owned(),
                            "/PID".to_owned(),
                            pid.to_string(),
                        ]
                    } else {
                        vec!["-KILL".to_owned(), pid.to_string()]
                    })
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                std::process::exit(i32::try_from(code).unwrap());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    // Persistent faults model a file still held after the RPC error, through fallback and shutdown cleanup.
    let pending: Arc<Mutex<Option<(Value, ChangedPath, bool)>>> = Arc::default();
    let output_pending = pending.clone();
    let block_record = plan["blockRecordDirectory"].as_str().map(PathBuf::from);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    // On Windows "server" names the started launcher until the first reply: by then its interpreter runs, and the
    // info is rewritten with that pid (the one Vibe's session lease records). Unix: the same pid, nothing changes.
    let mut info_after_reply =
        cfg!(windows).then(|| (plan_path.with_extension("info"), info.clone()));
    let out = std::thread::spawn(move || {
        let mut line = Vec::new();
        let mut stdout = std::io::stdout().lock();
        loop {
            line.clear();
            if output.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Some((file, mut info)) = info_after_reply.take() {
                info["server"] = json!(launchers::interpreter(pid));
                write(&file, &info);
            }
            if let Ok(frame) = serde_json::from_slice::<Value>(&line) {
                let message = frame.get("message").unwrap_or(&frame);
                if let (Some(dir), Some(sid)) = (
                    &block_record,
                    message
                        .pointer("/result/state/session/id")
                        .and_then(Value::as_str),
                ) {
                    std::fs::create_dir_all(dir.join(format!("{sid}.json.tmp"))).unwrap();
                }
                let mut guard = output_pending.lock().unwrap();
                if guard
                    .as_ref()
                    .is_some_and(|(id, _, persistent)| !persistent && message.get("id") == Some(id))
                {
                    drop(guard.take());
                }
            }
            if stdout
                .write_all(&line)
                .and_then(|()| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    });
    if plan["silentInitialize"].as_bool() == Some(true) {
        suspend(pid, true);
    }
    let mut input = child.stdin.take().unwrap();
    let mut reads: Vec<(Value, std::process::Child)> = Vec::new();
    for line in BufReader::new(std::io::stdin().lock()).split(b'\n') {
        let Ok(mut line) = line else { break };
        if plan["recordRequests"].as_bool() == Some(true) {
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(plan_path.with_extension("requests.jsonl"))
                .unwrap();
            log.write_all(&line).unwrap();
            log.write_all(b"\n").unwrap();
        }
        if let Ok(frame) = serde_json::from_slice::<Value>(&line) {
            let message = frame.get("message").unwrap_or(&frame);
            if frame.get("message") == Some(&Value::Null) {
                for (_, child) in reads
                    .iter_mut()
                    .filter(|(channel, _)| channel == &frame["channelId"])
                {
                    drop(child.stdin.take());
                    let _ = child.wait();
                }
            }
            if let Some(rules) = plan["onRequest"].as_array_mut() {
                for rule in rules {
                    if starts < rule["fromStart"].as_u64().unwrap_or(0)
                        || (rule["sessionId"].is_string()
                            && rule["sessionId"] != message["params"]["sessionId"])
                        || message["method"] != rule["method"]
                        || rule["times"].as_u64() == Some(0)
                    {
                        continue;
                    }
                    if let Some(times) = rule["times"].as_u64() {
                        rule["times"] = json!(times - 1);
                    }
                    if let Some(code) = rule["exit"].as_i64() {
                        // Trigger the same real-process kill as the control file, before forwarding this request.
                        write(&plan_path.with_extension("control"), &json!({"exit":code}));
                        loop {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    if let Some(kind) = rule["fault"].as_str() {
                        *pending.lock().unwrap() = Some((
                            message["id"].clone(),
                            ChangedPath::apply(kind, message),
                            rule["persistent"].as_bool() == Some(true),
                        ));
                    }
                    if let Some(ms) = rule["readDelayMs"].as_u64() {
                        reads.push((
                            frame["channelId"].clone(),
                            block_read(
                                message["params"]["sessionId"].as_str().unwrap(),
                                ms,
                                &plan_path,
                            ),
                        ));
                    }
                    if rule["suspendMs"].is_u64() || rule["suspendUntil"].is_string() {
                        suspend(pid, true);
                        resume_later(pid, rule);
                    }
                    break;
                }
            }
        }
        line.push(b'\n');
        if input.write_all(&line).and_then(|()| input.flush()).is_err() {
            break;
        }
    }
    if plan["ignoreEof"].as_bool() == Some(true) {
        std::thread::sleep(Duration::from_secs(600));
    }
    for (_, mut child) in reads {
        drop(child.stdin.take());
        let _ = child.wait();
    }
    drop(input);
    let status = child.wait().unwrap();
    let _ = out.join();
    // process::exit skips destructors, so restore the fault explicitly after Vibe has stopped.
    drop(pending.lock().unwrap().take());
    // Agent-class children have an independent group on Unix and are deliberately left for their owner.
    if let Some(child) = grandchild.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(child) = agent.as_mut() {
        let _ = child.try_wait();
    }
    let requested = exit_code.load(std::sync::atomic::Ordering::SeqCst);
    std::process::exit(if requested == i32::MIN {
        status.code().unwrap_or(1)
    } else {
        requested
    });
}
