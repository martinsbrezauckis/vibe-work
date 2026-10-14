//! `vw-agent-tools`: the binary Vibe starts for Vibe Work's session tools: `mcp` (per-session
//! stdio MCP server) and `kill-guard` (the `pre_tool` hook). The program lives in `vw_agent_tools::cli`.
//!
//! Depends on: the `vw_agent_tools` library. Called by: `vibe-app-server` (both modes).

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    vw_agent_tools::cli::run(&args)
}
