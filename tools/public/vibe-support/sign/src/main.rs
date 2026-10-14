//! `vibe-support-sign`: signs a release-channel file with the minisign updater key. The library
//! (`src/lib.rs`) holds the rules; this is the command line.
//!
//! Usage: `vibe-support-sign --key <path> --password <path> --pubkey <file with the base64 public key> <file>`
//! where `--key` and `--password` are pipes, e.g. `--key <(<password manager> show <entry>)`. Writes `<file>.sig`.
//! Prints only the output path. Exit codes: 0 signed, 2 usage, 1 refused or failed (the reason, never a secret).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: vibe-support-sign --key <path> --password <path> --pubkey <public key file> <file>"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (mut key, mut password, mut pubkey, mut file) = (None, None, None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--key") => key = args.next().map(PathBuf::from),
            Some("--password") => password = args.next().map(PathBuf::from),
            Some("--pubkey") => pubkey = args.next().map(PathBuf::from),
            Some(flag) if flag.starts_with("--") => return usage(),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => return usage(),
        }
    }
    let (Some(key), Some(password), Some(pubkey), Some(file)) = (key, password, pubkey, file)
    else {
        return usage();
    };
    let public = match std::fs::read_to_string(&pubkey) {
        Ok(text) if !text.trim().is_empty() => text,
        _ => {
            eprintln!(
                "the public key file {} is missing or empty",
                pubkey.display()
            );
            return ExitCode::from(1);
        }
    };
    match vibe_support_sign::sign_file(&file, &key, &password, &public) {
        Ok(out) => {
            println!("{}", out.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("not signed: {error}");
            ExitCode::from(1)
        }
    }
}
