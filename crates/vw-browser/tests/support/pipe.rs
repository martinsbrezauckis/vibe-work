//! The JSON-lines pipe to the Node page fixture (`page.mjs`): one request, one answer line, and the fixture's end.
//! Called by the fixture engine (`mod.rs`) and its CDP worker (`cdp.rs`).
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::Mutex;
use std::time::Duration;
use vw_preview::EngineError;

pub(crate) struct Pipe {
    pub(super) child: Child,
    pub(super) input: ChildStdin,
    pub(super) output: BufReader<ChildStdout>,
}

impl Drop for Pipe {
    fn drop(&mut self) {
        // Ask the fixture to close Chromium and remove its profile folder; kill it only if it does not end in time.
        let _ = writeln!(self.input, "{}", json!({"op":"close"}));
        let _ = self.input.flush();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn rpc_blocking(pipe: &Mutex<Pipe>, request: &Value) -> Result<Value, EngineError> {
    let mut pipe = pipe.lock().unwrap();
    writeln!(pipe.input, "{request}").map_err(|e| EngineError::Platform(e.to_string()))?;
    pipe.input
        .flush()
        .map_err(|e| EngineError::Platform(e.to_string()))?;
    let mut line = String::new();
    pipe.output
        .read_line(&mut line)
        .map_err(|e| EngineError::Platform(e.to_string()))?;
    let reply: Value =
        serde_json::from_str(&line).map_err(|e| EngineError::Platform(e.to_string()))?;
    if let Some(error) = reply["error"].as_str() {
        return Err(EngineError::Platform(error.to_owned()));
    }
    Ok(reply["value"].clone())
}
