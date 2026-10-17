//! Fake gh compiled by the integration harness; records invocations and deliberately echoes its fake token.
//! Depends only on std and test cwd files. No network or real login is used.

use std::{env, fs::OpenOptions, io::{self, Write}, process};

fn main() -> io::Result<()> {
    let argv: Vec<_> = env::args().skip(1).collect();
    writeln!(OpenOptions::new().create(true).append(true).open("runs")?, "{argv:?}")?;
    let token = env::var("GH_TOKEN").unwrap_or_default();
    println!("argv={argv:?}");
    println!("cwd={}", env::current_dir()?.display());
    println!("gh_token_set={}", !token.is_empty());
    let config = env::var("GH_CONFIG_DIR").unwrap_or_default();
    println!("config_empty={}", std::fs::read_dir(config)?.next().is_none());
    let inherited = ["GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GH_DEBUG", "GH_BROWSER", "GH_REPO", "HTTPS_PROXY"];
    println!("unexpected_env={}", inherited.iter().any(|key| env::var_os(key).is_some()));
    println!("token={token}/{token}");
    eprintln!("stderr-token={token}");
    io::stdout().write_all(b"binary=\xff\x00\n")?;
    let code = std::fs::read_to_string("exit-code").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    process::exit(code);
}
