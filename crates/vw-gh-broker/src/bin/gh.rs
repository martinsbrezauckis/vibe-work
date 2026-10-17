//! Credential-free PATH shim: sends argv and cwd to the local broker and preserves output/exit code.
//! Depends on vw-gh-broker; the app supplies only an endpoint in its environment.

use std::{
    io::{self, Write},
    process,
};
use vw_gh_broker::{ENDPOINT_ENV, Request, call};

fn main() {
    match forward() {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("gh broker: {error}");
            process::exit(1);
        }
    }
}

fn forward() -> io::Result<i32> {
    let endpoint = std::env::var(ENDPOINT_ENV)
        .map_err(|_| io::Error::other("no broker endpoint configured"))?;
    let request = Request {
        argv: std::env::args().skip(1).collect(),
        cwd: std::env::current_dir()?,
    };
    let reply = call(&endpoint, &request)?;
    io::stdout().lock().write_all(&reply.stdout)?;
    io::stderr().lock().write_all(&reply.stderr)?;
    Ok(reply.exit_code)
}
