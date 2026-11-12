//! Bounded helper supervision. Cancellation kills the process namespace/group and always schedules a reap.
use process_wrap::tokio::{ChildWrapper, CommandWrap};
use std::time::Duration;
use vw_video_types::{Code, Error};
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
    directory: Option<crate::staging::Directory>,
) -> Result<Vec<u8>, Error> {
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
    let mut child = Child {
        process: Some(command.spawn().map_err(|error| {
            Error::at(Code::WorkerStart, "helper sandbox could not start", error)
        })?),
        directory,
    };
    let process = child
        .process
        .as_mut()
        .ok_or(Error::new(Code::WorkerIsolation, "helper process"))?;
    let mut stdin = process
        .stdin()
        .take()
        .ok_or(Error::new(Code::WorkerFailed, "helper stdin"))?;
    let stdout = process
        .stdout()
        .take()
        .ok_or(Error::new(Code::WorkerFailed, "helper stdout"))?;
    let stderr = process
        .stderr()
        .take()
        .ok_or(Error::new(Code::WorkerFailed, "helper stderr"))?;
    #[cfg(target_os = "macos")]
    let pid = process
        .id()
        .ok_or(Error::new(Code::WorkerIsolation, "helper identity"))?;
    let work = async {
        let send = async {
            // A sandbox can refuse to start before reading its request. Collect its exit and
            // diagnosis before reporting the resulting broken pipe, within the same deadline.
            let mut input_result = stdin.write_all(&input).await;
            if input_result.is_ok() {
                // On Windows tokio wraps child pipes in a buffered blocking writer: `write_all` can return Ok
                // for a request the helper never read, and the broken pipe surfaces only on flush.
                input_result = stdin.flush().await;
            }
            drop(stdin);
            Ok::<_, Error>(input_result)
        };
        let wait = async {
            process
                .wait()
                .await
                .map_err(|error| Error::at(Code::WorkerFailed, "helper wait", error))
        };
        let (input_result, out, err, status) = tokio::try_join!(
            send,
            bounded(stdout, vw_agent_tools::wire::MAX_ANSWER),
            bounded(stderr, 16 * 1024),
            wait
        )?;
        if !status.success() {
            #[cfg(windows)]
            if status.code().map(i32::cast_unsigned) == Some(CPU_QUOTA) {
                return Err(Error::new(Code::Timeout, "video worker CPU time limit"));
            }
            return Err(Error::new(
                Code::WorkerFailed,
                format!(
                    "video helper terminated ({status}); {}",
                    String::from_utf8_lossy(&err)
                ),
            ));
        }
        input_result.map_err(|error| Error::at(Code::WorkerFailed, "helper input", error))?;
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
        .unwrap_or_else(|_| Err(Error::new(Code::Timeout, "video helper deadline")));
    reap(&mut child).await;
    result
}
// Kill even on success: no worker descendants are allowed to survive its response.
async fn reap(child: &mut Child) {
    // Retain ownership across the wait so cancellation can hand it to the drop reaper.
    if let Some(process) = child.process.as_mut() {
        let _ = process.start_kill();
        let _ = process.wait().await;
    }
    drop(child.process.take());
    ended();
}
#[cfg(target_os = "macos")]
async fn memory_limit(pid: u32) -> Error {
    loop {
        tokio::time::sleep(Duration::from_millis(20)).await;
        match vw_platform::video_sandbox::resident_bytes(pid) {
            Ok(bytes) if bytes <= 512 * 1024 * 1024 => (),
            Ok(_) => return Error::new(Code::Memory, "helper resident memory"),
            Err(error) => {
                // Give the concurrently polled wait future a chance to collect a just-exited child.
                tokio::time::sleep(Duration::from_millis(20)).await;
                return error;
            }
        }
    }
}
struct Child {
    process: Option<Box<dyn ChildWrapper>>,
    directory: Option<crate::staging::Directory>,
}
impl Drop for Child {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let directory = self.directory.take();
                runtime.spawn(async move {
                    let _ = process.wait().await;
                    drop(process);
                    ended();
                    // A cancelled extraction must keep its executable/input until the worker has exited.
                    drop(directory);
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
async fn bounded(pipe: impl tokio::io::AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, Error> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    pipe.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| Error::at(Code::WorkerFailed, "helper output read", error))?;
    if bytes.len() > limit {
        return Err(Error::new(Code::OutputLimit, "helper output"));
    }
    Ok(bytes)
}
#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "process_cleanup_tests.rs"]
mod cleanup_tests;
