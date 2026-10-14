//! What a `secret_run` consent binds to:
//! first commit: "the grant binds to the exact command"): the program file (path with links resolved, and its
//! content), the arguments in order, the working folder, which secret goes into which variable, and the content of
//! every file an argument names (the script `sh deploy.sh` runs). Checked again right before the program starts, so
//! a script the agent rewrites between the dialog and the run, or a program file swapped on `PATH`, is refused.
//!
//! In one sentence, for the user: "always" lets agents of this project run this same program with the same
//! arguments in the same folder, with the same files, putting the same secrets into the same variables.
//!
//! Not covered (stated, not hidden): files a program reads on its own (an `npm run deploy` reads `package.json`, which
//! no argument names) and what the program does with the value. The dialog shows the whole command so the user can
//! judge it; the output scrub is defense in depth only.
//!
//! The run uses the pin's argv ([`Pinned::args`]): an argument that named a file is passed as its canonical path, so
//! re-pointing a link after the consent changes nothing that runs; [`hold`] also refuses it.
//!
//! **The final check holds the files.** [`hold`] opens every pinned file,
//! hashes it through that open handle and keeps the handles until the run ends (`secret_run` drops them after the
//! program exits):
//! - Linux: the program is started through the held descriptor (`/proc/self/fd/<n>`, `secret_exec`), so a file
//!   renamed or moved over the program's path after the check cannot be what runs. Left open, bytes
//!   written into the same file in place between the hash and the start, and a `#!` program (the kernel cannot run a
//!   script through a descriptor that closes at exec, so it runs by path).
//! - Windows: the handles share reading only, so until the program has exited nobody can write, replace, rename or
//!   delete the program or a pinned script; the interpreter and the program still read them.
//! - macOS: no descriptor exec and no mandatory sharing modes: the path runs right after the check.
//!
//! A copy of the program in an app folder ("snapshot") was rejected: programs that find their own files next to
//! themselves stop working (uv's Python finds its standard library from its own path: a copy fails to start; Windows
//! loads a program's DLLs from its folder and has no way to keep the original path as `argv[0]`).
//!
//! Depends on: blake3, `serde_json`. Called by: `secret_run`, `secret_exec` ([`Held`]).
//! Invariant: blocking (reads and hashes files): callers use `spawn_blocking`.

use std::path::{Path, PathBuf};

use serde_json::json;

/// Largest file hashed (a program binary or a script); a larger one is refused.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// The pinned command.
#[derive(Clone, Debug)]
pub(crate) struct Pinned {
    /// The program with links resolved: what runs.
    pub(crate) program: PathBuf,
    /// The argv that runs: every argument (or `--option=` value) that named a file is replaced by that file's
    /// canonical path, so a link re-pointed after the consent cannot change what runs.
    pub(crate) args: Vec<String>,
    /// For each such argument: the path as written (joined to the folder) and the canonical path it resolved to.
    pub(crate) links: Vec<(PathBuf, PathBuf)>,
    /// The files checked (program first) with their BLAKE3 hashes.
    pub(crate) files: Vec<(PathBuf, String)>,
    /// One hash over everything the consent binds to (the grant's key).
    pub(crate) fingerprint: String,
}

fn hash_file(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("{} cannot be read ({e})", path.display()))?;
    hash_open(&file, path)
}

/// Hashes an open file from its start (the size cap from its own metadata, not the path's).
fn hash_open(file: &std::fs::File, path: &Path) -> Result<String, String> {
    let meta = file
        .metadata()
        .map_err(|e| format!("{} cannot be read ({e})", path.display()))?;
    if meta.len() > MAX_FILE {
        return Err(format!(
            "{} is too large to check (over 512 MiB)",
            path.display()
        ));
    }
    let mut hasher = blake3::Hasher::new();
    hasher
        .update_reader(file)
        .map_err(|e| format!("{} cannot be read ({e})", path.display()))?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Opens a pinned file for the final check. Windows: sharing reading only, so while the handle is open nobody can
/// write, replace, rename or delete the file (programs and interpreters only read it).
fn open_held(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// One argument that named an existing regular file (relative to `cwd` or absolute; also the value of
/// `--option=value`): the argument rewritten to the canonical path, and (as written, canonical).
type Named = (String, Option<(PathBuf, PathBuf)>);

fn resolve_arg(arg: &str, cwd: &Path) -> Result<Named, String> {
    let (prefix, candidate) = match arg.split_once('=').filter(|_| arg.starts_with('-')) {
        Some((option, value)) => (Some(option), value),
        None => (None, arg),
    };
    if candidate.is_empty() || candidate.starts_with('-') {
        return Ok((arg.to_owned(), None));
    }
    let written = cwd.join(candidate);
    if !written.is_file() {
        return Ok((arg.to_owned(), None));
    }
    let real = std::fs::canonicalize(&written)
        .map_err(|e| format!("{candidate} cannot be resolved ({e})"))?;
    let text = real.to_str().ok_or_else(|| {
        format!("{candidate} resolves to a path that is not valid text: not run with a secret")
    })?;
    let rewritten = match prefix {
        Some(option) => format!("{option}={text}"),
        None => text.to_owned(),
    };
    Ok((rewritten, Some((written, real))))
}

/// Pins a command. `bindings`: (variable, secret id, field).
///
/// # Errors
/// The program or a named file cannot be read, or is too large.
pub(crate) fn pin(
    program: &Path,
    args: &[String],
    cwd: &Path,
    bindings: &[(String, String, Option<String>)],
) -> Result<Pinned, String> {
    let program = std::fs::canonicalize(program)
        .map_err(|e| format!("{} cannot be resolved ({e})", program.display()))?;
    let mut files = vec![(program.clone(), hash_file(&program)?)];
    let mut run_args = Vec::with_capacity(args.len());
    let mut links = Vec::new();
    for arg in args {
        let (rewritten, link) = resolve_arg(arg, cwd)?;
        run_args.push(rewritten);
        if let Some((written, real)) = link {
            if !files.iter().any(|(p, _)| *p == real) {
                let hash = hash_file(&real)?;
                files.push((real.clone(), hash));
            }
            links.push((written, real));
        }
    }
    let mut sorted: Vec<_> = bindings
        .iter()
        .map(|(e, s, f)| (e.to_ascii_uppercase(), s, f))
        .collect();
    sorted.sort();
    let key = json!({
        "program": program,
        "args": args,
        "runArgs": run_args,
        "cwd": cwd,
        "bindings": sorted,
        "files": files,
    });
    let fingerprint = blake3::hash(key.to_string().as_bytes())
        .to_hex()
        .to_string();
    Ok(Pinned {
        program,
        args: run_args,
        links,
        files,
        fingerprint,
    })
}

/// The pinned files, held open from the final check until the run ends (see the module header).
#[derive(Debug)]
pub(crate) struct Held {
    /// The program, hashed through this very handle (Linux: the run starts it through it; elsewhere it is only held).
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only held open outside Linux")
    )]
    pub(crate) program: std::fs::File,
    /// The program is a `#!` script: it must run by path (Linux).
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "programs run by path outside Linux")
    )]
    pub(crate) script: bool,
    /// The other pinned files (named scripts), kept open for the run (Windows: nobody can change them meanwhile).
    _others: Vec<std::fs::File>,
}

/// The final check right before the start: opens every pinned file, checks it still has the content the user allowed
/// (hashed through the open handle) and keeps the handles ([`Held`]).
///
/// # Errors
/// The refusal for the agent, naming the file that changed or could not be opened (Windows: also a file another
/// program has open for writing), or an argument path that now points elsewhere.
pub(crate) fn hold(pinned: &Pinned) -> Result<Held, String> {
    // Defense in depth (the run uses the canonical paths anyway): a path as written that now resolves elsewhere is a
    // different command than the one allowed.
    for (written, real) in &pinned.links {
        if std::fs::canonicalize(written).ok().as_ref() != Some(real) {
            return Err(format!(
                "Not run: {} now points to another file than when the user allowed this command; ask again with \
                 secret_run",
                written.display()
            ));
        }
    }
    let mut files = Vec::with_capacity(pinned.files.len());
    for (path, hash) in &pinned.files {
        let file = open_held(path).map_err(|e| {
            format!(
                "Not run: {} could not be opened for the final check ({e}); ask again with secret_run",
                path.display()
            )
        })?;
        if hash_open(&file, path).ok().as_ref() != Some(hash) {
            return Err(format!(
                "Not run: {} changed after the user allowed this command; ask again with secret_run",
                path.display()
            ));
        }
        files.push(file);
    }
    if files.is_empty() {
        return Err("Not run: the program was not checked".to_owned());
    }
    let program = files.remove(0);
    let script = is_script(&program);
    Ok(Held {
        program,
        script,
        _others: files,
    })
}

/// Whether `file` starts with `#!` (read at offset 0 without moving anything that matters: the handle is only used
/// to start the program after this).
fn is_script(file: &std::fs::File) -> bool {
    use std::io::{Read as _, Seek as _};
    let mut head = [0_u8; 2];
    let mut file = file;
    file.seek(std::io::SeekFrom::Start(0)).is_ok()
        && file.read_exact(&mut head).is_ok()
        && head == *b"#!"
}
