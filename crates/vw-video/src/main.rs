//! Restricted helper entry point. One bounded request on stdin and one bounded reply on stdout.
use std::io::{Read, Write};
fn run() -> Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    {
        // The core launcher mounts only its input and this executable, at these fixed names.
        if !std::path::Path::new("/input").is_file() || !std::path::Path::new("/worker").is_file() {
            return Err("backend_unavailable: helper requires its sandbox".into());
        }
        vw_platform::video::limits::restrict()?;
    }
    #[cfg(not(target_os = "linux"))]
    return Err("backend_unavailable: signed native helper sandbox is unavailable".into());
    #[allow(unreachable_code)]
    {
        let mut input = Vec::new();
        std::io::stdin()
            .take((vw_agent_tools::wire::MAX_LINE + 1) as u64)
            .read_to_end(&mut input)
            .map_err(|_| "decode_failed: request read")?;
        if input.len() > vw_agent_tools::wire::MAX_LINE {
            return Err("limit_exceeded: request bytes".into());
        }
        let job: vw_video::protocol::Job =
            serde_json::from_slice(&input).map_err(|_| "decode_failed: request shape")?;
        if job.request.path != "/input" {
            return Err("decode_failed: input capability".into());
        }
        let reply = vw_video::worker::extract(&job)?;
        vw_video::protocol::validate(&job, &reply)?;
        let bytes = serde_json::to_vec(&reply).map_err(|_| "decode_failed: reply serialization")?;
        if bytes.len() > vw_agent_tools::wire::MAX_ANSWER {
            return Err("limit_exceeded: reply bytes".into());
        }
        Ok(bytes)
    }
}
fn main() {
    let result = run().unwrap_or_else(|error| {
        serde_json::to_vec(&serde_json::json!({"error":error})).unwrap_or_default()
    });
    let _ = std::io::stdout().write_all(&result);
}
