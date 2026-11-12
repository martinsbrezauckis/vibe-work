//! Restricted helper entry point. One bounded request on stdin and one bounded reply on stdout.
use std::io::{Read, Write};
use vw_video_types::{Code, Error};
fn run() -> Result<Vec<u8>, Error> {
    #[cfg(not(target_os = "linux"))]
    let path = native_setup()?;
    #[cfg(target_os = "linux")]
    let path = "/input".to_owned();
    #[cfg(target_os = "linux")]
    {
        // The core launcher mounts only its input and this executable, at these fixed names.
        if !std::path::Path::new("/input").is_file() || !std::path::Path::new("/worker").is_file() {
            return Err(Error::new(
                Code::WorkerIsolation,
                "helper requires its sandbox",
            ));
        }
        vw_platform::video::limits::restrict()?;
    }
    {
        let mut input = Vec::new();
        std::io::stdin()
            .take((vw_agent_tools::wire::MAX_LINE + 1) as u64)
            .read_to_end(&mut input)
            .map_err(|error| Error::at(Code::WorkerFailed, "request read", error))?;
        if input.len() > vw_agent_tools::wire::MAX_LINE {
            return Err(Error::new(Code::OutputLimit, "request bytes"));
        }
        let job: vw_video::protocol::Job = serde_json::from_slice(&input)
            .map_err(|error| Error::at(Code::WorkerFailed, "request shape", error))?;
        if job.request.path != path {
            return Err(Error::new(Code::WorkerFailed, "input capability"));
        }
        let reply = vw_video::worker::extract(&job)?;
        vw_video::protocol::validate(&job, &reply)?;
        let bytes = serde_json::to_vec(&reply)
            .map_err(|error| Error::at(Code::WorkerFailed, "reply serialization", error))?;
        if bytes.len() > vw_agent_tools::wire::MAX_ANSWER {
            return Err(Error::new(Code::OutputLimit, "reply bytes"));
        }
        Ok(bytes)
    }
}

#[cfg(not(target_os = "linux"))]
fn native_setup() -> Result<String, Error> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 || args[1] != "--worker" {
        return Err(Error::new(
            Code::WorkerIsolation,
            "helper requires its sandbox",
        ));
    }
    let path = std::path::Path::new(&args[2]);
    #[cfg(windows)]
    vw_platform::video::isolation::verify()?;
    #[cfg(target_os = "macos")]
    vw_platform::video::isolation::restrict(path)?;
    path.to_str()
        .map(str::to_owned)
        .ok_or(Error::new(Code::WorkerFailed, "input path encoding"))
}
fn main() {
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() == 3 && args[1] == "--sandbox-launch" {
            let status = std::env::current_exe().and_then(|helper| {
                vw_platform::video::isolation::launch(&helper, std::path::Path::new(&args[2]))
            });
            match status {
                Ok(code) => std::process::exit(code.cast_signed()),
                Err(error) => {
                    let error = Error::new(Code::WorkerIsolation, error);
                    let bytes = serde_json::to_vec(&vw_video::protocol::Failure { error })
                        .unwrap_or_default();
                    let _ = std::io::stdout().write_all(&bytes);
                }
            }
            return;
        }
    }
    let result = run().unwrap_or_else(|error| {
        serde_json::to_vec(&vw_video::protocol::Failure { error }).unwrap_or_default()
    });
    let _ = std::io::stdout().write_all(&result);
}
