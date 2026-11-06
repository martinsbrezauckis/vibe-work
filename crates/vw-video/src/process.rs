//! Bounded helper supervision. Cancellation kills the process namespace/group and always schedules a reap.
use process_wrap::tokio::{ChildWrapper, CommandWrap};
use std::time::Duration;
/// `STATUS_QUOTA_EXCEEDED`: Windows ended the worker at its Job's CPU-time limit (`vw_platform` video isolation).
/// The work was too large for the limit; it is a timeout with a next step, not a broken worker.
#[cfg(windows)]
const CPU_QUOTA: u32 = 0xC000_0044;
/// Executes a prepared sandbox command with a deadline and bounded output.
/// # Errors
/// Spawn, deadline, output flood, cancellation or nonzero exit.
pub async fn capture(
    mut command: CommandWrap,
    deadline: Duration,
    input: Vec<u8>,
) -> Result<Vec<u8>, String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    command
        .command_mut()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    command.wrap(process_wrap::tokio::KillOnDrop);
    let mut child =
        Child(Some(command.spawn().map_err(
            |_| "backend_unavailable: helper sandbox could not start",
        )?));
    let process = child
        .0
        .as_mut()
        .ok_or("backend_unavailable: helper process")?;
    let mut stdin = process
        .stdin()
        .take()
        .ok_or("decode_failed: helper stdin")?;
    let stdout = process
        .stdout()
        .take()
        .ok_or("decode_failed: helper stdout")?;
    let stderr = process
        .stderr()
        .take()
        .ok_or("decode_failed: helper stderr")?;
    #[cfg(target_os = "macos")]
    let pid = process.id().ok_or("backend_unavailable: helper identity")?;
    let work = async {
        let send = async {
            stdin
                .write_all(&input)
                .await
                .map_err(|_| "decode_failed: helper input")?;
            drop(stdin);
            Ok::<_, String>(())
        };
        let wait = async {
            process
                .wait()
                .await
                .map_err(|_| "decode_failed: helper wait".to_owned())
        };
        let ((), out, err, status) = tokio::try_join!(
            send,
            bounded(stdout, vw_agent_tools::wire::MAX_ANSWER),
            bounded(stderr, 16 * 1024),
            wait
        )?;
        if !status.success() {
            #[cfg(windows)]
            if status.code().map(i32::cast_unsigned) == Some(CPU_QUOTA) {
                return Err("timeout: video worker CPU time limit".into());
            }
            return Err(if err.starts_with(b"bwrap:") {
                "backend_unavailable: Linux sandbox namespaces or media access denied".into()
            } else {
                format!("decode_failed: video helper terminated ({status})")
            });
        }
        Ok(out)
    };
    #[cfg(target_os = "macos")]
    let work = async {
        tokio::select! {
            result = work => result,
            error = memory_limit(pid) => Err(error),
        }
    };
    let result = tokio::time::timeout(deadline, work)
        .await
        .unwrap_or_else(|_| Err("timeout: video helper deadline".into()));
    // Kill even on success: no worker descendants are allowed to survive its response.
    if let Some(mut process) = child.0.take() {
        let _ = process.start_kill();
        let _ = process.wait().await;
        ended();
    }
    result
}
#[cfg(target_os = "macos")]
async fn memory_limit(pid: u32) -> String {
    loop {
        tokio::time::sleep(Duration::from_millis(20)).await;
        match vw_platform::video_sandbox::resident_bytes(pid) {
            Ok(bytes) if bytes <= 512 * 1024 * 1024 => (),
            Ok(_) => return "limit_exceeded: helper resident memory".into(),
            Err(error) => {
                // Give the concurrently polled wait future a chance to collect a just-exited child.
                tokio::time::sleep(Duration::from_millis(20)).await;
                return error;
            }
        }
    }
}
struct Child(Option<Box<dyn ChildWrapper>>);
impl Drop for Child {
    fn drop(&mut self) {
        if let Some(mut process) = self.0.take() {
            let _ = process.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = process.wait().await;
                    ended();
                });
            }
        }
    }
}
/// After a helper is reaped. A killed Windows bootstrap (deadline, cancellation) cannot delete its own
/// `AppContainer` profile, so the parent removes profiles whose bootstrap no longer runs.
fn ended() {
    #[cfg(windows)]
    vw_platform::video_sandbox::sweep();
}
async fn bounded(pipe: impl tokio::io::AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    pipe.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "decode_failed: helper output read")?;
    if bytes.len() > limit {
        return Err("limit_exceeded: helper output".into());
    }
    Ok(bytes)
}
#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
