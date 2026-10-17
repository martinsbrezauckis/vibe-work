//! Owns a temporary Mistral upload and its cleanup independently of the requesting future.
//! Depends on work HTTP helpers and Tokio; image edits hold the receipt until Mistral stops using the source.
//! Dropping a receipt releases cleanup; the task also finishes an in-flight upload if its caller is cancelled.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use super::{ApiCall, WorkError, header_file_name, json_call};

/// Receives a plain notice when deletion remains unconfirmed after all retries. No key or response body.
pub type CleanupNotice = Arc<dyn Fn(String) + Send + Sync>;

const DELETE_ATTEMPTS: usize = 3;
const DELETE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct TemporaryFile {
    pub(crate) id: String,
    released: Option<oneshot::Sender<()>>,
    done: oneshot::Receiver<Result<(), WorkError>>,
}

impl TemporaryFile {
    /// Starts the owner before sending the upload, so cancellation cannot discard an acquired file id.
    pub(crate) async fn upload(
        client: &reqwest::Client,
        base: &str,
        key: &str,
        body: (Vec<u8>, String),
        name: &str,
        timeout: Duration,
        notice: CleanupNotice,
    ) -> Result<Self, WorkError> {
        let (client, base, key) = (
            client.clone(),
            base.to_owned(),
            Zeroizing::new(key.to_owned()),
        );
        let name = header_file_name(name);
        let (ready, uploaded) = oneshot::channel();
        let (released, release) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        tokio::spawn(async move {
            let answer = json_call(
                &client,
                &base,
                ApiCall {
                    method: reqwest::Method::POST,
                    path: "/v1/files".to_owned(),
                    body: Some(body),
                    timeout,
                    key: &key,
                },
            )
            .await;
            let id = answer.and_then(|answer| {
                answer
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| {
                        !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    })
                    .map(str::to_owned)
                    .ok_or(WorkError::Shape("no file id"))
            });
            let id = match id {
                Ok(id) => id,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            let _ = ready.send(Ok(id.clone()));
            // Both ordinary completion and cancellation drop/send the receipt's release channel.
            let _ = release.await;
            let result = cleanup(&client, &base, &key, &id, &name).await;
            if let Err(error) = &result {
                notice(error.to_string());
            }
            let _ = done.send(result);
        });
        let id = uploaded
            .await
            .map_err(|_| WorkError::Shape("the upload owner stopped"))??;
        Ok(Self {
            id,
            released: Some(released),
            done: finished,
        })
    }

    pub(crate) async fn finish(mut self) -> Result<(), WorkError> {
        drop(self.released.take());
        self.done
            .await
            .map_err(|_| WorkError::Shape("the cleanup owner stopped"))?
    }
}

async fn cleanup(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    id: &str,
    name: &str,
) -> Result<(), WorkError> {
    for attempt in 0..DELETE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(if attempt == 1 {
                250
            } else {
                1_000
            }))
            .await;
        }
        let auth = crate::auth::bearer(key).ok_or(WorkError::MalformedKey)?;
        let response = client
            .delete(format!("{}/v1/files/{id}", base.trim_end_matches('/')))
            .header(reqwest::header::AUTHORIZATION, auth)
            .timeout(DELETE_TIMEOUT)
            .send()
            .await;
        if response.is_ok_and(|response| {
            response.status().is_success() || response.status().as_u16() == 404
        }) {
            return Ok(());
        }
    }
    Err(WorkError::CleanupFailed(format!(
        "The temporary source image \"{name}\" (Mistral file {id}) could not be deleted after {DELETE_ATTEMPTS} attempts. \
         It may still be in your Mistral account. Delete it in Mistral's console."
    )))
}
