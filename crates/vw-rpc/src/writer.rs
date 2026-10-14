//! The single writer task: serializes every outbound frame onto the server's stdin.
//!
//! Depends on: `shared`. Called by: `connection` (spawned once per connection).
//! Invariants: frames are written whole and in queue order; every write is bounded by
//! `write_timeout` (a server that stops reading stdin would otherwise block us forever, the
//! ShellX `PROVIDER_SETUP_STDIN_TIMEOUT_MS` scar); a failed or timed-out write declares the
//! server dead and ends the task, which closes stdin.

use std::sync::Arc;

use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::debug;

use crate::error::DeathCause;
use crate::shared::{Shared, WriterCmd};

pub(crate) async fn run<W>(mut writer: W, mut queue: mpsc::Receiver<WriterCmd>, shared: Arc<Shared>)
where
    W: AsyncWrite + Unpin,
{
    let limit = shared.config.write_timeout;
    while let Some(cmd) = queue.recv().await {
        match cmd {
            WriterCmd::Frame(bytes) => {
                let write = async {
                    writer.write_all(&bytes).await?;
                    writer.flush().await
                };
                match tokio::time::timeout(limit, write).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        shared.mark_dead(DeathCause::WriteFailed(error.to_string()), None);
                        return;
                    }
                    Err(_) => {
                        shared.mark_dead(DeathCause::WriteTimedOut { after: limit }, None);
                        return;
                    }
                }
            }
            WriterCmd::Close => {
                debug!("closing server stdin");
                // Best effort: the drop below closes the pipe even if shutdown fails.
                let _ = tokio::time::timeout(limit, writer.shutdown()).await;
                return;
            }
        }
    }
}
