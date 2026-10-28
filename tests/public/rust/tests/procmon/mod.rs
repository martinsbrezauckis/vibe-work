//! Process and scratch helpers for isolated test processes and loopback listeners.
//! A drop guard stops processes and removes their disposable folders.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use vw_procmon::ProcessRef;
use vw_procmon::{ProcessInfo, SelfInfo, SessionContext, Snapshot, SocketListener, StartToken};

/// A fresh, canonical temp folder unique to this test run, removed (with its content) when dropped, also
/// when the test panics. Declare it before the processes that run in it, so they are killed first.
pub struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// See [`Scratch`].
pub fn scratch(name: &str) -> Scratch {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Under the shared real-Vibe test root: the Linux test sandbox binds only that root, so a session folder in
    // the system temp folder would be invisible to Vibe there (the sandbox exposes only its fixture tree).
    let dir = vw_public_tests::test_paths::root()
        .join("vw-procmon-tests")
        .join(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let canonical = dir.canonicalize().unwrap();
    // Windows: canonicalize adds the `\\?\` verbatim prefix, which a child's working directory must not carry.
    Scratch(PathBuf::from(
        canonical
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned(),
    ))
}

/// A session context with one folder.
pub fn ctx(id: &str, folder: &Path) -> SessionContext {
    SessionContext {
        session_id: id.to_owned(),
        folders: vec![folder.to_path_buf()],
        background: Vec::new(),
    }
}

/// A test port: `offset` (1..=999) inside this test process's own block of 1000 ports. Each test binary
/// claims a free block from 20000-31999 (outside every default ephemeral range) by holding a listener on
/// the block's first port for its whole lifetime, so parallel worktrees never share ports (saw a
/// `port_free` collision with the old fixed 28100-28999). `VW_TEST_PORT_BASE` overrides the base.
pub fn port(offset: u16) -> u16 {
    assert!((1..=999).contains(&offset), "test port offsets are 1..=999");
    port_base() + offset
}

fn port_base() -> u16 {
    static CLAIM: std::sync::OnceLock<(u16, Option<TcpListener>)> = std::sync::OnceLock::new();
    CLAIM
        .get_or_init(|| {
            if let Some(base) = std::env::var("VW_TEST_PORT_BASE")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
            {
                return (base, None);
            }
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
                ^ std::process::id();
            for step in 0..12u32 {
                let block = u16::try_from(seed.wrapping_add(step) % 12).unwrap();
                let base = 20_000 + block * 1000;
                if let Ok(claim) = TcpListener::bind((Ipv4Addr::LOCALHOST, base)) {
                    return (base, Some(claim));
                }
            }
            panic!("no free test port block in 20000-31999");
        })
        .0
}

/// Waits until something accepts on 127.0.0.1:`port`.
pub fn wait_listening(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("nothing listens on {port} after 15 s");
}

/// True when nothing accepts on 127.0.0.1:`port`.
pub fn port_free(port: u16) -> bool {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err()
}

/// The Python interpreter for listener processes: `VW_TEST_PYTHON`, else `python3` (Unix) / `python`
/// (Windows, where `python3` may be the Store alias stub).
pub fn python() -> String {
    std::env::var("VW_TEST_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".to_owned()
        } else {
            "python3".to_owned()
        }
    })
}

/// A process a test started, in its own process group (Unix) / console process group (Windows); on drop
/// its own group is SIGKILLed (Unix; a process that joined another group is killed alone) or the tree is
/// force-terminated (Windows `taskkill /T /F`).
pub struct Spawned {
    pub child: Child,
    pub pgid: u32,
}

impl Spawned {
    /// The pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Its identity (pid + start token) right now.
    pub fn identity(&self) -> ProcessRef {
        vw_procmon::identity(self.pid()).expect("the spawned process is alive")
    }

    /// True while the process runs (reaps it when it has exited, so a zombie counts as gone).
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill, killpg};
            use nix::unistd::Pid;
            // Only a process that leads its own group takes the group with it. One that joined another
            // process's group (`spawn_in(.., Some(group))`, the "intruder" of vw_procmon_live) is killed
            // alone: killing its group SIGKILLed the group's leader too, so a test that keeps the leader
            // running saw it die whenever the kill landed before its check (Windows round, 6 of 12 runs
            // at load ~40).
            if let Ok(raw) = i32::try_from(self.pgid)
                && raw > 1
            {
                if self.pgid == self.pid() {
                    let _ = killpg(Pid::from_raw(raw), Signal::SIGKILL);
                } else if let Ok(pid) = i32::try_from(self.pid()) {
                    let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &self.pid().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.wait();
    }
}

/// Starts `program args` in `dir`, output discarded: Unix in a new process group (or in `group` when given),
/// Windows in a new console process group without a window (`group` must be `None`).
pub fn spawn_in(dir: &Path, program: &str, args: &[&str], group: Option<u32>) -> Spawned {
    spawn_in_with_stdout(dir, program, args, group, Stdio::null())
}

fn spawn_in_with_stdout(
    dir: &Path,
    program: &str,
    args: &[&str],
    group: Option<u32>,
    stdout: Stdio,
) -> Spawned {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(group.map_or(0, |g| i32::try_from(g).unwrap()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        assert!(group.is_none(), "Windows has no joinable process groups");
        // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        command.creation_flags(0x0000_0200 | 0x0800_0000);
    }
    let child = command
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
    let pgid = group.unwrap_or_else(|| child.id());
    Spawned { child, pgid }
}

// A connection can reach another process after a bind failure. Only the child's own output proves
// that it bound the requested port; an EOF means it exited before reaching its ready line.
fn ready_line(mut spawned: Spawned) -> Result<(Spawned, String), String> {
    let stdout = spawned.child.stdout.take().expect("captured stdout");
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout)
            .read_line(&mut line)
            .map(|read| (read, line));
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(Duration::from_secs(60));
    match result {
        Ok(Ok((read, line))) if read > 0 => {
            reader.join().expect("ready reader finished");
            Ok((spawned, line))
        }
        other => {
            // Stop our child before joining the pipe reader, including a child that never reports ready.
            drop(spawned);
            reader.join().expect("ready reader stopped");
            Err(format!("server did not report a bound port: {other:?}"))
        }
    }
}

fn ready(spawned: Spawned, expected: &str) -> Result<Spawned, String> {
    let (spawned, line) = ready_line(spawned)?;
    if !line.starts_with(expected) {
        return Err(format!("unexpected server ready line: {line:?}"));
    }
    Ok(spawned)
}

fn reported_port(line: &str) -> u16 {
    line.trim()
        .strip_prefix("READY ")
        .expect("server reported its port")
        .parse()
        .expect("server reported a numeric port")
}

/// Starts a shell stand-in whose Python child binds port zero and reports its held port.
/// The child may lead its own group, or share `group` with the shell for group-refusal tests.
#[cfg(unix)]
pub fn shell_server(dir: &Path, separate_server_group: bool, group: Option<u32>) -> (Spawned, u16) {
    let own_group = if separate_server_group {
        "os.setpgid(0,0);"
    } else {
        ""
    };
    let code = format!(
        "import os,http.server;{own_group}\
         server=http.server.ThreadingHTTPServer((\"127.0.0.1\",0),http.server.SimpleHTTPRequestHandler);\
         print(\"READY\",server.server_address[1],flush=True);server.serve_forever()"
    );
    let script = format!("python3 -c '{code}' & wait");
    let spawned = spawn_in_with_stdout(dir, "sh", &["-c", &script], group, Stdio::piped());
    let (spawned, line) = ready_line(spawned).expect("stand-in server bound a port");
    (spawned, reported_port(&line))
}

fn try_http_server(dir: &Path, port: u16) -> Result<Spawned, String> {
    let port_text = port.to_string();
    let spawned = spawn_in_with_stdout(
        dir,
        &python(),
        &["-m", "http.server", &port_text, "--bind", "127.0.0.1"],
        None,
        Stdio::piped(),
    );
    ready(spawned, "Serving HTTP on 127.0.0.1 port ")
}

/// `python -m http.server <port> --bind 127.0.0.1` in `dir`, confirmed by its own ready line.
pub fn http_server(dir: &Path, port: u16) -> Spawned {
    try_http_server(dir, port).unwrap_or_else(|e| panic!("server on {port}: {e}"))
}

// The child tries the range itself and reports the socket it still owns. This avoids handing a
// previously released number across the process boundary.
fn server_in_range(
    dir: &Path,
    range: std::ops::RangeInclusive<u16>,
    with_child: bool,
    ignore_term: bool,
) -> (Spawned, u16) {
    let [first, last] = [range.start().to_string(), range.end().to_string()];
    let child = if with_child {
        "subprocess.Popen([sys.executable,'-c','import time; time.sleep(600)'],stdout=subprocess.DEVNULL)"
    } else {
        "pass"
    };
    let signal = if ignore_term {
        "import signal; signal.signal(signal.SIGTERM,signal.SIG_IGN)"
    } else {
        "pass"
    };
    // A taken port is EADDRINUSE, except on Windows, where HTTPServer's SO_REUSEADDR makes it WSAEACCES (errno 13:
    // "An attempt was made to access a socket in a way forbidden by its access permissions").
    let code = format!(
        r"import errno,http.server,subprocess,sys
for port in range(int(sys.argv[1]),int(sys.argv[2])+1):
    try:
        server=http.server.ThreadingHTTPServer(('127.0.0.1',port),http.server.SimpleHTTPRequestHandler)
        break
    except OSError as exc:
        if exc.errno not in (errno.EADDRINUSE, errno.EACCES): raise
else:
    raise RuntimeError('no port in range can bind')
{signal}
{child}
print('READY',server.server_address[1],flush=True)
server.serve_forever()"
    );
    let spawned = spawn_in_with_stdout(
        dir,
        &python(),
        &["-c", &code, &first, &last],
        None,
        Stdio::piped(),
    );
    let (spawned, line) = ready_line(spawned).expect("server bound a port in range");
    (spawned, reported_port(&line))
}

/// Starts a server in `range`, with the child reporting the port it actually holds.
pub fn http_server_in_range(dir: &Path, range: std::ops::RangeInclusive<u16>) -> (Spawned, u16) {
    server_in_range(dir, range, false, false)
}

/// Starts a server that ignores TERM on a directly bound port in `range` (Unix escalation tests).
#[cfg(unix)]
pub fn http_server_ignoring_term_in_range(
    dir: &Path,
    range: std::ops::RangeInclusive<u16>,
) -> (Spawned, u16) {
    server_in_range(dir, range, false, true)
}

/// A Python HTTP server on `port` with extra command-line arguments it ignores (`sys.argv[2..]`), to check
/// that no view or ledger ever shows arguments.
pub fn http_server_with_args(dir: &Path, port: u16, extra: &[&str]) -> Spawned {
    let code = "import sys,http.server,socketserver;\
                server=socketserver.TCPServer(('127.0.0.1',int(sys.argv[1])),http.server.SimpleHTTPRequestHandler);\
                print('READY',flush=True);server.serve_forever()";
    let port_text = port.to_string();
    let mut args = vec!["-c", code, &port_text];
    args.extend_from_slice(extra);
    let spawned = spawn_in_with_stdout(dir, &python(), &args, None, Stdio::piped());
    ready(spawned, "READY").unwrap_or_else(|e| panic!("server on {port}: {e}"))
}

/// A Python HTTP server on `port` that replaces its own argv[0] with `argv0` (`os.execv`), so a view that
/// trusted the command line would show it.
pub fn http_server_with_argv0(dir: &Path, port: u16, argv0: &str) -> Spawned {
    let server = "import sys,http.server,socketserver;\
                  server=socketserver.TCPServer(('127.0.0.1',int(sys.argv[1])),http.server.SimpleHTTPRequestHandler);\
                  print('READY',flush=True);server.serve_forever()";
    let code =
        format!("import os,sys; os.execv(sys.executable, [{argv0:?}, '-c', {server:?}, '{port}'])");
    let spawned = spawn_in_with_stdout(dir, &python(), &["-c", &code], None, Stdio::piped());
    ready(spawned, "READY").unwrap_or_else(|e| panic!("server on {port}: {e}"))
}

/// A Python HTTP server on `port` that first starts one child Python process (sleeping), so stopping it
/// must end a tree of two. Portable (no shell).
pub fn http_server_with_child(dir: &Path, port: u16) -> Spawned {
    try_http_server_with_child(dir, port).unwrap_or_else(|e| panic!("server on {port}: {e}"))
}

fn try_http_server_with_child(dir: &Path, port: u16) -> Result<Spawned, String> {
    let code = format!(
        "import subprocess,sys,http.server,socketserver;\
         server=socketserver.TCPServer(('127.0.0.1',{port}),http.server.SimpleHTTPRequestHandler);\
         subprocess.Popen([sys.executable,'-c','import time; time.sleep(600)'],stdout=subprocess.DEVNULL);\
         print('READY',flush=True);server.serve_forever()"
    );
    let spawned = spawn_in_with_stdout(dir, &python(), &["-c", &code], None, Stdio::piped());
    ready(spawned, "READY")
}

/// Starts the server and its child on the first port it can actually bind in `range`.
pub fn http_server_with_child_in_range(
    dir: &Path,
    range: std::ops::RangeInclusive<u16>,
) -> (Spawned, u16) {
    server_in_range(dir, range, true, false)
}

/// Registers `spawned` for `session` in the monitor's registry (a fact, like a terminal or a
/// `process_start` would), with its process group on Unix.
pub fn register(
    monitor: &vw_procmon::ProcMon,
    session: &str,
    spawned: &Spawned,
    class: vw_procmon::ProcessClass,
) {
    let group = if cfg!(unix) {
        vw_procmon::ProcessGroup::Unix { pgid: spawned.pgid }
    } else {
        vw_procmon::ProcessGroup::None
    };
    let _ = monitor.registry().register(vw_procmon::OwnProcess {
        session_id: Some(session.to_owned()),
        process: spawned.identity(),
        group,
        class,
        label: "test process".to_owned(),
        keep_running: false,
    });
}

/// A synthetic process row (same user, alive).
pub fn proc_row(pid: u32, ppid: u32, start: u64, cwd: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        ppid: Some(ppid),
        pgid: Some(pid),
        start: Some(StartToken(start)),
        same_user: Some(true),
        name: format!("proc{pid}"),
        cmdline: vec![format!("proc{pid}")],
        cwd: Some(PathBuf::from(cwd)),
        exe: Some(PathBuf::from(format!("/usr/bin/proc{pid}"))),
        alive: true,
    }
}

/// A synthetic snapshot from rows and `(port, pid)` listeners on 127.0.0.1.
pub fn snapshot(rows: Vec<ProcessInfo>, listeners: &[(u16, Option<u32>)]) -> Snapshot {
    let processes: BTreeMap<u32, ProcessInfo> = rows.into_iter().map(|p| (p.pid, p)).collect();
    Snapshot {
        listeners: listeners
            .iter()
            .map(|(port, pid)| SocketListener {
                address: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: *port,
                pid: *pid,
                same_user: pid
                    .and_then(|p| processes.get(&p))
                    .and_then(|p| p.same_user),
            })
            .collect(),
        processes,
        me: SelfInfo {
            pid: 1,
            pgid: Some(1),
        },
        took: Duration::ZERO,
    }
}
