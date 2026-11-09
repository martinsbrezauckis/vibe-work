//! Private command-output sink for vault creation; never constructs a tool-output JSON value.
//! Uses the existing process environment and monitor, with group/job cleanup on cancellation and bounded zeroizing reads.
use super::secret_exec::Exec;
use crate::core::Shared;
use process_wrap::tokio::{ChildWrapper, KillOnDrop};
use std::{ffi::OsString, process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};
use vw_procmon::{OwnProcess, ProcessClass, ProcessGroup, ProcessRef};
use vw_vault::SecretString;
use zeroize::Zeroizing;

const FAILED: &str =
    "The command did not produce a complete secret; check the command and try again.";
const STOPPED: &str =
    "The capture ended or its access changed; ask the user before starting it again.";

struct Owned {
    child: Box<dyn ChildWrapper>,
    shared: std::sync::Weak<Shared>,
    process: Option<ProcessRef>,
    armed: bool,
}
impl Drop for Owned {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.start_kill();
        }
        if let (Some(shared), Some(process)) = (self.shared.upgrade(), self.process) {
            shared.procmon.registry().unregister_process(process);
        }
    }
}

/// Drains a private pipe; overflow and read errors are failures, never partial successful output.
async fn read(mut pipe: impl AsyncRead + Unpin) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let mut output = Zeroizing::new(Vec::with_capacity(vw_vault::project_secret::MAX_VALUE + 2));
    let mut chunk = Zeroizing::new([0u8; 512]);
    loop {
        let count = pipe.read(&mut *chunk).await.map_err(|_| FAILED)?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > vw_vault::project_secret::MAX_VALUE + 2 {
            return Err(FAILED);
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

fn spawn(
    exec: &Exec<'_>,
    values: &[(String, SecretString)],
) -> Result<Box<dyn ChildWrapper>, String> {
    let mut env: Vec<(OsString, OsString)> = exec
        .process_env
        .iter()
        .filter(|(name, _)| {
            !values
                .iter()
                .any(|(key, _)| name.to_string_lossy().eq_ignore_ascii_case(key))
        })
        .cloned()
        .collect();
    env.extend(
        values
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value.expose()))),
    );
    vw_supervisor::isolate_credentials(&mut env);
    let mut command = vw_platform::process_command::hidden_wrap(exec.program, |cmd| {
        cmd.args(exec.args)
            .current_dir(exec.cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        command.wrap(process_wrap::tokio::JobObject);
    }
    command.wrap(KillOnDrop);
    command.spawn().map_err(|_| FAILED.into())
}

pub(super) async fn execute(
    shared: &Arc<Shared>,
    exec: &Exec<'_>,
    values: &[(String, SecretString)],
    single_line: bool,
    allowed: impl Fn() -> bool + Send + Sync,
) -> Result<SecretString, String> {
    if !allowed() {
        return Err(STOPPED.into());
    }
    let child = spawn(exec, values)?;
    let pid = child.id();
    let process = pid.and_then(vw_procmon::identity);
    let mut owned = Owned {
        child,
        shared: Arc::downgrade(shared),
        process,
        armed: true,
    };
    let process = process.ok_or(FAILED)?;
    let _ = shared.procmon.registry().register(OwnProcess {
        session_id: Some(exec.session_id.into()),
        process,
        group: if cfg!(unix) {
            ProcessGroup::Unix { pgid: process.pid }
        } else {
            ProcessGroup::None
        },
        class: ProcessClass::Agent,
        label: "secret_capture_command".into(),
        keep_running: false,
    });
    let stdout = owned.child.stdout().take().ok_or(FAILED)?;
    let completed = async {
        tokio::try_join!(
            async { owned.child.wait().await.map_err(|_| FAILED) },
            read(stdout),
        )
    };
    let cancelled = async {
        loop {
            if !allowed() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let result = tokio::select! {
        biased;
        () = cancelled => Err(STOPPED),
        result = tokio::time::timeout(exec.timeout, completed) => result.map_err(|_| FAILED).and_then(|r| r),
    };
    let (status, output) = match result {
        Ok(result) => result,
        Err(reason) => {
            let _ = owned.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), owned.child.wait()).await;
            return Err(reason.into());
        }
    };
    if !status.success() || !allowed() {
        return Err(FAILED.into());
    }
    // Unix waitpid cannot wait for an orphaned grandchild. Even with closed pipes, a live group member
    // means incomplete execution. Windows' JobObject wait above waits for the whole job instead.
    #[cfg(unix)]
    {
        let monitor = shared.procmon.clone();
        let clean = tokio::task::spawn_blocking(move || {
            monitor
                .scan()
                .map(|s| !s.processes.values().any(|p| p.alive && p.pgid == pid))
        })
        .await
        .map_err(|_| FAILED)?
        .map_err(|_| FAILED)?;
        if !clean {
            return Err(FAILED.into());
        }
    }
    owned.armed = false;
    let text = std::str::from_utf8(&output).map_err(|_| FAILED)?;
    let text = if single_line {
        let text = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        if text.contains(['\r', '\n']) {
            return Err(FAILED.into());
        }
        text
    } else {
        text
    };
    if text.is_empty() || text.len() > vw_vault::project_secret::MAX_VALUE {
        return Err(FAILED.into());
    }
    Ok(SecretString::exact(text))
}

#[cfg(test)]
mod tests {
    use super::read;
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::{AsyncRead, ReadBuf};

    struct FailsAfterPrefix(bool);
    impl AsyncRead for FailsAfterPrefix {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.0 {
                return Poll::Ready(Err(io::Error::other("synthetic private pipe failure")));
            }
            self.0 = true;
            buffer.put_slice(b"a private prefix that is not a complete secret");
            Poll::Ready(Ok(()))
        }
    }
    #[tokio::test]
    async fn interrupted_read_never_commits_a_partial_value() {
        assert!(read(FailsAfterPrefix(false)).await.is_err());
    }
}
