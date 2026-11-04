//! Bounded helper supervision. Cancellation kills the process namespace/group and always schedules a reap.
use process_wrap::tokio::{ChildWrapper, CommandWrap};
use std::time::Duration;
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
            return Err(if err.starts_with(b"bwrap:") {
                "backend_unavailable: Linux sandbox namespaces or media access denied"
            } else {
                "decode_failed: video helper terminated"
            }
            .into());
        }
        Ok(out)
    };
    let result = tokio::time::timeout(deadline, work)
        .await
        .unwrap_or_else(|_| Err("timeout: video helper deadline".into()));
    // Kill even on success: no worker descendants are allowed to survive its response.
    if let Some(mut process) = child.0.take() {
        let _ = process.start_kill();
        let _ = process.wait().await;
    }
    result
}
struct Child(Option<Box<dyn ChildWrapper>>);
impl Drop for Child {
    fn drop(&mut self) {
        if let Some(mut process) = self.0.take() {
            let _ = process.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = process.wait().await;
                });
            }
        }
    }
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
#[cfg(unix)]
mod tests {
    use super::*;
    fn shell(script: &str) -> CommandWrap {
        CommandWrap::with_new("/bin/sh", |c| {
            c.args(["-c", script]);
        })
    }
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn cancelled_helper_is_killed_and_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let command = CommandWrap::with_new("/bin/sh", |c| {
            c.args(["-c", "printf '%s' $$ > \"$1\"; exec sleep 30", "worker"])
                .arg(&pidfile);
        });
        let task = tokio::spawn(capture(command, Duration::from_secs(35), Vec::new()));
        let started = std::time::Instant::now();
        while !pidfile.exists() && started.elapsed() < Duration::from_secs(2) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(pidfile).unwrap();
        task.abort();
        let _ = task.await;
        let proc = std::path::Path::new("/proc").join(pid);
        let started = std::time::Instant::now();
        while proc.exists() && started.elapsed() < Duration::from_secs(2) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!proc.exists(), "cancelled helper remains alive or unreaped");
    }
    #[tokio::test]
    async fn helper_deadline_is_external() {
        assert_eq!(
            capture(shell("sleep 10"), Duration::from_millis(20), Vec::new())
                .await
                .unwrap_err(),
            "timeout: video helper deadline"
        );
    }
    #[tokio::test]
    async fn helper_output_flood_is_bounded() {
        assert_eq!(
            capture(shell("yes overflow"), Duration::from_secs(3), Vec::new())
                .await
                .unwrap_err(),
            "limit_exceeded: helper output"
        );
    }
}
