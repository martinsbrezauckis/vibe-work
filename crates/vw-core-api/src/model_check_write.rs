//! Conditional model-facts writes while Vibe can read the same config file.
//! Uses the private atomic writer; `model_check` keeps unrelated and concurrently changed bytes intact.
use std::path::Path;

use super::failed;
use crate::error::CoreError;

pub(super) async fn replace(path: &Path, before: &str, after: &str) -> Result<bool, CoreError> {
    replace_with(
        path,
        before,
        after,
        cfg!(windows),
        vw_platform::replace_private,
    )
    .await
}

async fn replace_with(
    path: &Path,
    before: &str,
    after: &str,
    windows: bool,
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), vw_platform::PrivateFileError>,
) -> Result<bool, CoreError> {
    if after.parse::<toml::Table>().is_err() {
        return Err(failed("Vibe Work's model config cannot be edited safely"));
    }
    // Vibe's Python config reader can hold a handle without FILE_SHARE_DELETE. Atomic
    // replacement can report ERROR_ACCESS_DENIED or ERROR_SHARING_VIOLATION until it finishes.
    // Wait at most 375 ms (25 + 50 + 100 + 200), only for those Windows errors. Recheck
    // the original bytes before EVERY attempt so a concurrent edit is never overwritten.
    let mut pauses = [25, 50, 100, 200].into_iter();
    loop {
        if crate::default_model::read(path)? != before {
            return Ok(false);
        }
        let error = match write(path, after.as_bytes()) {
            Ok(()) => return Ok(true),
            Err(error) => error,
        };
        let sharing = windows
            && matches!(
                &error,
                vw_platform::PrivateFileError::Io { action: "replace file", source, .. }
                    if matches!(source.raw_os_error(), Some(5 | 32))
            );
        if sharing && let Some(ms) = pauses.next() {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            continue;
        }
        // Preserve the OS diagnosis in logs; the visible state remains short.
        tracing::warn!(%error, "model config replacement failed");
        return Err(failed("Vibe Work's model config could not be saved"));
    }
}

#[cfg(test)]
#[path = "model_check_write_tests.rs"]
mod tests;
