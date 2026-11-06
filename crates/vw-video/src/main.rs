//! Restricted helper entry point. One bounded request on stdin and one bounded reply on stdout.
use std::io::{Read, Write};
fn run() -> Result<Vec<u8>, String> {
    #[cfg(not(target_os = "linux"))]
    let path = native_setup()?;
    #[cfg(target_os = "linux")]
    let path = "/input".to_owned();
    #[cfg(target_os = "linux")]
    {
        // The core launcher mounts only its input and this executable, at these fixed names.
        if !std::path::Path::new("/input").is_file() || !std::path::Path::new("/worker").is_file() {
            return Err("backend_unavailable: helper requires its sandbox".into());
        }
        vw_platform::video::limits::restrict()?;
    }
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
        if job.request.path != path {
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

#[cfg(not(target_os = "linux"))]
fn native_setup() -> Result<String, String> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 || args[1] != "--worker" {
        return Err("backend_unavailable: helper requires its sandbox".into());
    }
    let path = std::path::Path::new(&args[2]);
    #[cfg(windows)]
    vw_platform::video::isolation::verify()?;
    #[cfg(target_os = "macos")]
    vw_platform::video::isolation::restrict(path)?;
    path.to_str()
        .map(str::to_owned)
        .ok_or("decode_failed: input path encoding".into())
}
fn main() {
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() == 3 && args[1] == "--sandbox-launch" {
            let status = std::env::current_exe().and_then(|helper| {
                vw_platform::video::isolation::launch(&helper, std::path::Path::new(&args[2]))
            });
            if let Ok(code) = status {
                // Pass the worker's exit code through bit for bit: a Job limit's NTSTATUS (0xC0000044 for CPU
                // time) does not fit an i32 and once became an indistinct 1.
                std::process::exit(code.cast_signed());
            }
            let _ = std::io::stdout()
                .write_all(br#"{"error":"backend_unavailable: Windows media sandbox denied"}"#);
            return;
        }
    }
    let result = run().unwrap_or_else(|error| {
        serde_json::to_vec(&serde_json::json!({"error":error})).unwrap_or_default()
    });
    let _ = std::io::stdout().write_all(&result);
}
