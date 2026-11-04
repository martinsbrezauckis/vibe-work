//! Credential-free feedback POST; fixed build-time HTTPS endpoint, no redirects, bounded response and timeout.
use crate::{CoreError, ErrorCode, FeedbackReport};
use std::time::Duration;

pub(crate) fn endpoint() -> Option<url::Url> {
    let url = url::Url::parse(
        option_env!("VW_FEEDBACK_URL").unwrap_or("https://feedback.u1c.lv/v1/reports"),
    )
    .ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/v1/reports")
        .then_some(url)
}
fn failed() -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        "Delivery could not be confirmed. Your report is kept; try Send again.",
    )
}
pub(crate) async fn send(url: url::Url, report: &FeedbackReport) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec(report).map_err(|_| failed())?;
    if bytes.len() > 16384 {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "The report is too long. Shorten it and review again.",
        ));
    }
    let client = vw_mistral::https_client().map_err(|_| failed())?;
    let mut reply = client
        .post(url)
        .timeout(Duration::from_secs(20))
        .header("Content-Type", "application/json")
        .body(bytes)
        .send()
        .await
        .map_err(|_| failed())?;
    if reply.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "Too many reports. Your report is kept; try again later.",
        ));
    }
    if !matches!(reply.status().as_u16(), 200 | 201) {
        return Err(failed());
    }
    let mut body = Vec::new();
    while let Some(chunk) = reply.chunk().await.map_err(|_| failed())? {
        if body.len() + chunk.len() > 1024 {
            return Err(failed());
        }
        body.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| failed())?;
    if value["stored"] != true || value["id"].as_str() != Some(report.id.as_str()) {
        return Err(failed());
    }
    Ok(())
}
