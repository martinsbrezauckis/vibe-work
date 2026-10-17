//! One newline-delimited JSON request/reply per local connection; used by broker and shim.
//! Serde owns framing and parsing. Only argv and cwd travel to the broker, never a token.

use interprocess::local_socket::{Stream, prelude::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};

/// A client invocation, with no executable, environment or credential override.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Arguments following `gh`, already split by the invoking shell.
    pub argv: Vec<String>,
    /// Working directory used for repository discovery.
    pub cwd: PathBuf,
}

/// Captured gh output, or a refusal (exit code 1 and a plain reason on stderr).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    /// gh's exit code; a signal termination uses shell convention on Unix.
    pub exit_code: i32,
    /// Exact stdout bytes, after token redaction.
    pub stdout: Vec<u8>,
    /// Exact stderr bytes, after token redaction.
    pub stderr: Vec<u8>,
}

impl Reply {
    pub(crate) fn refused(reason: &str) -> Self {
        Self {
            exit_code: 1,
            stdout: Vec::new(),
            stderr: format!("gh broker: {reason}\n").into_bytes(),
        }
    }
}

/// Forward an invocation over the local endpoint. This client never obtains the token.
///
/// # Errors
/// Returns an error when the broker is unavailable or its reply is not valid JSON.
pub fn call(endpoint: &str, request: &Request) -> io::Result<Reply> {
    let mut stream = Stream::connect(super::local::name(endpoint)?)?;
    send(&mut stream, request)?;
    receive(&mut BufReader::new(stream))
}

pub(crate) fn send(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

pub(crate) fn receive<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<T> {
    let mut frame = Vec::new();
    reader.read_until(b'\n', &mut frame)?;
    Ok(serde_json::from_slice(&frame)?)
}
