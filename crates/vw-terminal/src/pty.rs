//! Opens a PTY (`ConPTY` on Windows) and starts the shell in it with exactly our environment.
//!
//! Depends on: portable-pty, `shell`. Called by: `terminal` (each start and restart).
//! Invariants:
//! - The child's environment is exactly `env` (`env_clear` first: portable-pty's builder otherwise
//!   starts from the whole process environment and, on Windows, the registry environment).
//! - The slave side is dropped right after spawn, so the reader sees end-of-file once the last
//!   process holding the terminal exits.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::shell::ShellCommand;

/// A started shell and the PTY handles the terminal keeps.
pub(crate) struct Spawned {
    /// Master side: resize, and (on Windows) the pseudo console that must close after exit.
    pub(crate) master: Box<dyn MasterPty + Send>,
    /// Output of the shell (a duplicated handle; reading blocks).
    pub(crate) reader: Box<dyn Read + Send>,
    /// Input to the shell (writing blocks when the PTY input buffer is full).
    pub(crate) writer: Box<dyn Write + Send>,
    /// The shell process.
    pub(crate) child: Box<dyn Child + Send + Sync>,
}

/// The PTY size for `cols` x `rows`.
pub(crate) fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Opens a PTY of `cols` x `rows` and starts `shell` in `cwd` with `env`.
pub(crate) fn spawn(
    shell: &ShellCommand,
    cwd: &Path,
    env: &[(OsString, OsString)],
    cols: u16,
    rows: u16,
) -> Result<Spawned, String> {
    let pair = native_pty_system()
        .openpty(size(cols, rows))
        .map_err(|e| format!("cannot open a terminal device: {e}"))?;
    let mut command = CommandBuilder::new(shell.program.as_os_str());
    command.args(&shell.args);
    command.env_clear();
    for (name, value) in env {
        command.env(name, value);
    }
    command.cwd(cwd.as_os_str());
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| format!("cannot start {}: {e}", shell.program.display()))?;
    drop(pair.slave);
    let handles = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("cannot read the terminal: {e}"))
        .and_then(|reader| {
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| format!("cannot write to the terminal: {e}"))?;
            Ok((reader, writer))
        });
    let (reader, writer) = match handles {
        Ok(handles) => handles,
        Err(error) => {
            // Never leave a shell running that nothing can talk to.
            let _ = child.kill();
            return Err(error);
        }
    };
    Ok(Spawned {
        master: pair.master,
        reader,
        writer,
        child,
    })
}
