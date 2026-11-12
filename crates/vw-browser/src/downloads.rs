//! Download management using the preview's chat-scoped staging and delivery records.
//! Called inside the browser Stop scope; never accepts a filesystem path or another chat's download id.
//! Depends on: preview download records.
//! Called by: browser host actions.
use crate::{
    ActError, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};

/// One validated file download request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Start a URL transfer.
    Download(String),
    /// List recent downloads of this chat.
    List,
    /// Wait for exactly one attributed download.
    Wait {
        /// An opaque id from a start, click receipt or list.
        id: Option<String>,
        /// Bounded wait in milliseconds.
        timeout: u64,
    },
    /// Verify that a completed artifact was delivered to the chat folder.
    Save(String),
}

impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let id = || {
            args["downloadId"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .map(str::to_owned)
                .ok_or_else(|| {
                    "Use a downloadId returned by this chat's download or list.".to_owned()
                })
        };
        Ok(match args["action"].as_str() {
            Some("files.download") => {
                let url=args["url"].as_str().filter(|s|s.len()<=8192).and_then(|s|url::Url::parse(s).ok()).filter(|u|matches!(u.scheme(),"http"|"https")&&u.host_str().is_some()&&u.username().is_empty()&&u.password().is_none()).ok_or("Download needs an HTTP(S) URL without credentials; use a vault login to sign in.")?;
                Self::Download(url.to_string())
            }
            Some("files.list_downloads") => Self::List,
            Some("files.wait_download") => Self::Wait {
                id: args.get("downloadId").map(|_| id()).transpose()?,
                timeout: match args.get("timeoutMs") {
                    None => 15_000,
                    Some(v) => v
                        .as_u64()
                        .filter(|n| *n <= 20_000)
                        .ok_or("Download wait timeoutMs must be between 0 and 20000.")?,
                },
            },
            Some("files.save_download") => Self::Save(id()?),
            _ => return Err("Unknown download action.".into()),
        })
    }
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Download(_) => "files.download",
            Self::List => "files.list_downloads",
            Self::Wait { .. } => "files.wait_download",
            Self::Save(_) => "files.save_download",
        }
    }
}

impl Browser {
    pub(crate) async fn downloads(&self, page: &str, request: &Request) -> Result<Value, ActError> {
        match request {
            Request::Download(url) => {
                let url = url::Url::parse(url).map_err(|_| refused("Download URL is invalid."))?;
                let id = self
                    .service
                    .download_url(page, url)
                    .await
                    .map_err(|e| failed(crate::driver::preview_text(&e)))?;
                Ok(
                    json!({"Outcome":"Download started. Wait for its completion before using the file.","downloadId":id}),
                )
            }
            Request::List => Ok(
                json!({"Outcome":"This chat's downloads listed.","untrustedPageContent":{"downloads":self.service.chat_downloads(page)}}),
            ),
            Request::Wait { id, timeout } => {
                let id = if let Some(id) = id {
                    id.clone()
                } else {
                    let rows = self.service.chat_downloads(page);
                    if rows.len() != 1 {
                        return Err(refused(
                            "DOWNLOAD_AMBIGUOUS: choose a downloadId from files.list_downloads for this chat.",
                        ));
                    }
                    rows[0]["id"].as_str().unwrap_or_default().to_owned()
                };
                let start = tokio::time::Instant::now();
                loop {
                    let mut row = self.download_record(page, &id)?;
                    if row["finished"] == true {
                        let outcome = if row["state"] == "complete" {
                            "Download finished and was saved in this chat's downloads folder."
                        } else {
                            "Download was interrupted; no complete artifact was saved."
                        };
                        return Ok(json!({"Outcome":outcome,"untrustedPageContent":row}));
                    }
                    if start.elapsed().as_millis() >= u128::from(*timeout) {
                        row["timedOut"] = json!(true);
                        return Ok(
                            json!({"Outcome":"Download is still in progress at the bounded wait deadline. Completion has not been confirmed.","untrustedPageContent":row}),
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            Request::Save(id) => {
                let row = self.download_record(page, id)?;
                if row["state"] != "complete" {
                    return Err(refused(
                        "The download is not complete; use files.wait_download before saving its artifact.",
                    ));
                }
                Ok(
                    json!({"Outcome":"Download saved in this chat's downloads folder.","untrustedPageContent":row}),
                )
            }
        }
    }
    fn download_record(&self, page: &str, id: &str) -> Result<Value, ActError> {
        self.service.chat_downloads(page).into_iter().find(|r|r["id"]==id).ok_or_else(||refused("DOWNLOAD_AMBIGUOUS: this chat has no download with that id. List this chat's downloads before trying again."))
    }
}
