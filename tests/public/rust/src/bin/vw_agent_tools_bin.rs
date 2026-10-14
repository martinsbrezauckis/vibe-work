//! The `vw-agent-tools` program built inside the public test crate, so tests can start it by
//! `env!("CARGO_BIN_EXE_vw_agent_tools_bin")`. Same code as the shipped binary
//! (`vw_agent_tools::cli::run`).

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    vw_agent_tools::cli::run(&args)
}
