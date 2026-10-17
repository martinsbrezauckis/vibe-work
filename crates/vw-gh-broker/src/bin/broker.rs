//! Standalone broker process: trusted caller pipes one token line to stdin; stdout advertises the endpoint.
//! Depends on clap and vw-gh-broker. Credentials never enter command arguments or the shim environment.

use clap::{Arg, Command, value_parser};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    process,
};
use vw_gh_broker::Broker;
use zeroize::Zeroizing;

fn main() {
    if let Err(error) = serve() {
        eprintln!("gh broker: {error}");
        process::exit(1);
    }
}

fn serve() -> io::Result<()> {
    let matches = Command::new("vw-gh-broker")
        .arg(
            Arg::new("gh")
                .long("gh")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .try_get_matches()
        .map_err(|_| {
            io::Error::other(
                "usage: vw-gh-broker --gh <absolute real gh path>; pipe one token line to stdin",
            )
        })?;
    let executable = matches
        .get_one::<PathBuf>("gh")
        .ok_or_else(|| io::Error::other("missing real gh path"))?;
    let mut token = Zeroizing::new(String::new());
    io::stdin().lock().read_line(&mut token)?;
    let broker = Broker::bind(token.trim_end_matches(['\n', '\r']).to_owned(), executable)?;
    println!("{}", broker.endpoint());
    io::stdout().flush()?;
    broker.serve()
}
