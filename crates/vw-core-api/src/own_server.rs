//! Recognizes Vibe only along the launcher image sequence rooted at the supervisor's spawned PID.
//! Uses OS process identities and the selected environment; lease owners below the actual server stay foreign.
use std::path::{Path, PathBuf};
use vw_procmon::{ProcessInfo, Snapshot};

fn runs_program(args: &[String], program: &Path) -> bool {
    // Vibe 3 sets its process title before serving. setproctitle replaces argv on Unix,
    // leaving empty padding arguments. The caller also checks the executable and exact
    // launcher ancestry, so a title alone never establishes ownership.
    args.iter().take(2).any(|arg| Path::new(arg) == program)
        || (args.first().is_some_and(|arg| arg == "vibe-app-server")
            && args.iter().skip(1).all(String::is_empty))
        || runs_adapter(args, program)
}

/// The supervisor's child-Stop launch of `program`: `<python> -P -c <exactly our adapter> <program> ...`. On Windows
/// the title is not visible in the command line, and the environment's `python.exe` starts the base Python with
/// this same command line, so this is how both processes name the server.
fn runs_adapter(args: &[String], program: &Path) -> bool {
    matches!(args, [_, isolate, dash_c, code, server, ..]
        if isolate == "-P"
            && dash_c == "-c"
            && code == vw_supervisor::CHILD_STOP_ADAPTER
            && Path::new(server) == program)
}

/// `launchers`: the executables of the supervisor's configured launcher argv
/// ([`vw_supervisor::Supervisor::launcher_programs`]); empty when it spawns the server itself.
pub(crate) fn is_server(pid: u32, leader: u32, program: &Path, launchers: &[PathBuf]) -> bool {
    if pid == leader {
        return true;
    }
    let Ok(mut snapshot) = vw_procmon::snapshot() else {
        return false;
    };
    let Some(chain) = ancestors(&snapshot, pid, leader) else {
        return false;
    };
    vw_procmon::fill_cmdline(&mut snapshot, &chain);
    owns(
        &snapshot,
        chain,
        leader,
        program,
        launchers,
        &launcher_images(program),
    )
}

/// Whether the lease owner at `chain[0]` (walked up to `leader`) is our server. Without a configured launcher the
/// image sequence starts at the spawned process. With one, the spawned process and the processes it starts down to
/// the server must each run one of the launcher's own executables; the image sequence starts directly below them.
fn owns(
    snapshot: &Snapshot,
    mut chain: Vec<u32>,
    leader: u32,
    program: &Path,
    launchers: &[PathBuf],
    images: &[PathBuf],
) -> bool {
    if launchers.is_empty() {
        return matches_chain(snapshot, &chain, leader, program, images);
    }
    let is_launcher = |pid: u32| {
        snapshot.alive(pid).is_some_and(|process| {
            process.same_user == Some(true)
                && process
                    .exe
                    .as_deref()
                    .is_some_and(|exe| launchers.iter().any(|image| same_file(exe, image)))
        })
    };
    let mut wrappers = 0;
    while chain.len() > 1 && chain.last().is_some_and(|&pid| is_launcher(pid)) {
        chain.pop();
        wrappers += 1;
    }
    wrappers > 0
        && wrappers <= launchers.len()
        && chain
            .last()
            .is_some_and(|&top| matches_chain(snapshot, &chain, top, program, images))
}

fn same_file(exe: &Path, image: &Path) -> bool {
    exe == image || std::fs::canonicalize(exe).ok().as_deref() == Some(image)
}

fn ancestors(snapshot: &Snapshot, pid: u32, leader: u32) -> Option<Vec<u32>> {
    let mut chain = Vec::new();
    let mut process = snapshot.alive(pid)?;
    loop {
        if chain.contains(&process.pid) || chain.len() >= 32 {
            return None;
        }
        chain.push(process.pid);
        if process.pid == leader {
            return Some(chain);
        }
        process = snapshot.parent_of(process)?;
        if !process.alive {
            return None;
        }
    }
}

/// The images of our server from the spawned end down: on Windows the entry point, the environment's Python and its
/// optional base Python; elsewhere the program, or for a `#!` script the interpreter it names (the process image).
/// When the supervisor starts the server through the child-Stop adapter ([`vw_supervisor::adapter_python`]), the
/// spawned process is the environment's Python itself, so the entry point is not in the sequence.
fn launcher_images(program: &Path) -> Vec<PathBuf> {
    let python = if cfg!(windows) {
        vw_supervisor::server_python(program)
    } else {
        None
    };
    let base = python.as_deref().and_then(base_python);
    image_order(
        program,
        vw_supervisor::adapter_python(program),
        python,
        base,
    )
}

/// [`launcher_images`] from its parts: the adapter's interpreter (when the server runs through it), and on Windows
/// the environment's Python and its base Python.
fn image_order(
    program: &Path,
    adapter: Option<PathBuf>,
    python: Option<PathBuf>,
    base: Option<PathBuf>,
) -> Vec<PathBuf> {
    let first = match adapter {
        // Windows reports the environment's `python.exe` as it is spelled (no `exec`, no symlink); on Unix the
        // process image is the interpreter the environment's `python` link resolves to.
        Some(python) if cfg!(windows) => python,
        Some(python) => mac_app(std::fs::canonicalize(&python).unwrap_or(python)),
        None => script_interpreter(program).map_or_else(|| program.to_owned(), mac_app),
    };
    let mut images = vec![first];
    for image in python.into_iter().chain(base) {
        if !images.contains(&image) {
            images.push(image);
        }
    }
    images
}

/// On macOS a framework Python (Homebrew, python.org) is the stub `Versions/<v>/bin/python3.x`, which execs
/// `Versions/<v>/Resources/Python.app/Contents/MacOS/Python` in the same process: that app is the image the OS
/// reports for our server. Any other interpreter (and every other OS) keeps its own image.
fn mac_app(interpreter: PathBuf) -> PathBuf {
    if !cfg!(target_os = "macos") {
        return interpreter;
    }
    let version = interpreter
        .parent()
        .filter(|bin| bin.file_name().is_some_and(|name| name == "bin"))
        .and_then(Path::parent);
    let app = version
        .filter(|version| {
            version.parent().is_some_and(|versions| {
                versions.file_name().is_some_and(|name| name == "Versions")
                    && versions
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == "Python.framework")
            })
        })
        .and_then(|version| {
            std::fs::canonicalize(version.join("Resources/Python.app/Contents/MacOS/Python")).ok()
        });
    app.unwrap_or(interpreter)
}

/// The canonical base interpreter named by the `home` line of the environment's `pyvenv.cfg` (Windows layout).
fn base_python(python: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(python.parent()?.parent()?.join("pyvenv.cfg")).ok()?;
    let home = text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "home").then(|| PathBuf::from(value.trim()))
    })?;
    std::fs::canonicalize(home.join("python.exe")).ok()
}

/// The canonical interpreter of a Unix `#!` script; `None` on Windows (no `exec`: its launchers are separate
/// processes, matched by [`launcher_images`]) and for a native executable.
fn script_interpreter(program: &Path) -> Option<PathBuf> {
    use std::io::Read;
    if cfg!(windows) {
        return None;
    }
    let mut head = [0u8; 512];
    let read = std::fs::File::open(program).ok()?.read(&mut head).ok()?;
    let line = head[..read]
        .strip_prefix(b"#!")?
        .split(|&b| b == b'\n')
        .next()?;
    let interpreter = std::str::from_utf8(line).ok()?.split_whitespace().next()?;
    std::fs::canonicalize(interpreter).ok()
}

fn matches_chain(
    snapshot: &Snapshot,
    chain: &[u32],
    leader: u32,
    program: &Path,
    images: &[PathBuf],
) -> bool {
    // No arbitrary named descendant counts as a launcher. The chain must reach the spawned process,
    // then follow the actual image order: entry point, environment Python, optional base Python.
    if chain.last() != Some(&leader) || chain.len() > images.len() {
        return false;
    }
    chain.iter().rev().zip(images).all(|(pid, image)| {
        snapshot
            .alive(*pid)
            .is_some_and(|process| matches_image(process, image, program))
    })
}

fn matches_image(process: &ProcessInfo, image: &Path, program: &Path) -> bool {
    process.same_user == Some(true)
        && runs_program(&process.cmdline, program)
        && process
            .exe
            .as_deref()
            .is_some_and(|exe| same_file(exe, image))
}

#[cfg(test)]
#[path = "own_server_tests.rs"]
mod tests;
