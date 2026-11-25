//! The config edits on the forms Vibe itself writes.
//!
//! Vibe's inline MCP list is edited in place, including when it already contains entries.

use super::*;

const VIBE_WRITTEN: &str =
    "active_model = \"m\"\nmcp_servers = []\n\n[[models]]\nname = \"m\"\nalias = \"m\"\n";

fn home_with(config: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), config).unwrap();
    (dir, home)
}

fn servers_in(home: &Path) -> Vec<toml::Value> {
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let table: toml::Table = text.parse().unwrap();
    assert_eq!(table["active_model"].as_str(), Some("m"), "{text}");
    assert_eq!(table["models"].as_array().map(Vec::len), Some(1), "{text}");
    table["mcp_servers"].as_array().unwrap().clone()
}

#[test]
fn a_key_server_is_added_to_an_empty_inline_server_list() {
    let (_dir, home) = home_with(VIBE_WRITTEN);
    add_entry(&home, "echokey", "http://127.0.0.1:4100/vw-mcp/x").unwrap();
    let list = servers_in(&home);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"].as_str(), Some("echokey"));
    assert_eq!(list[0]["auth"]["api_key_env"].as_str(), Some(TOKEN_ENV));
}

#[test]
fn an_imported_server_is_added_to_an_empty_inline_server_list() {
    let (_dir, home) = home_with(VIBE_WRITTEN);
    let server = vw_import::mcp::Server::Http {
        url: "https://docs.example.com/mcp".to_owned(),
        headers: Vec::new(),
    };
    add_imported(&home, "docs", &server, false).unwrap();
    let list = servers_in(&home);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"].as_str(), Some("docs"));
}

#[test]
fn an_imported_command_server_is_disabled_in_its_first_saved_config_while_off() {
    let (dir, home) = home_with(VIBE_WRITTEN);
    let server = vw_import::mcp::Server::Stdio {
        command: "fixture".to_owned(),
        args: Vec::new(),
        env: Vec::new(),
    };
    add_imported(&home, "local", &server, true).unwrap();
    let list = servers_in(&home);
    assert_eq!(list[0]["transport"].as_str(), Some("stdio"));
    assert_eq!(list[0]["disabled"].as_bool(), Some(true));
    let settings = dir.path().join("settings.json");
    crate::command_pause::remember_paused(Some(&settings), "local");
    assert_eq!(
        crate::app_settings::paused_command_servers(&settings),
        vec!["local".to_owned()]
    );
}

#[test]
fn a_key_server_is_added_to_a_non_empty_inline_list_without_rewriting_other_rows() {
    let text = "active_model = \"m\"\nmcp_servers = [{ name = \"a\", transport = \"stdio\", command = \"a\" }]\n\n[[models]]\nname = \"m\"\nalias = \"m\"\n";
    let (_dir, home) = home_with(text);
    add_entry(&home, "echokey", "http://127.0.0.1:4100/vw-mcp/x").unwrap();
    let after = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(after.contains("{ name = \"a\", transport = \"stdio\", command = \"a\" }"));
    assert!(after.contains("mcp_servers = ["), "{after}");
    assert!(!after.contains("[[mcp_servers]]"), "{after}");
    let rows = servers_in(&home);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["name"].as_str(), Some("echokey"));
}

#[test]
fn an_imported_command_is_appended_paused_to_a_non_empty_inline_list() {
    let text = "active_model = \"m\"\nmcp_servers = [{ name = \"web\", transport = \"http\", url = \"https://x.invalid\" }]\n\n[[models]]\nname = \"m\"\nalias = \"m\"\n";
    let (_dir, home) = home_with(text);
    let server = vw_import::mcp::Server::Stdio {
        command: "/bin/echo".to_owned(),
        args: Vec::new(),
        env: Vec::new(),
    };
    add_imported(&home, "local", &server, true).unwrap();
    let after = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(
        after.contains("{ name = \"web\", transport = \"http\", url = \"https://x.invalid\" }")
    );
    assert!(after.contains("mcp_servers = ["), "{after}");
    assert_eq!(servers_in(&home)[1]["disabled"].as_bool(), Some(true));
}

#[test]
fn an_inline_forwarder_rewrites_its_port_without_changing_other_bytes() {
    let text = "active_model = \"m\"\nmcp_servers = [{ name = \"key\", transport = \"streamable-http\", url = \"http://127.0.0.1:4100/vw-mcp/vibe-work-mcp-0123456789abcdef\" }]\n\n[[models]]\nname = \"m\"\nalias = \"m\"\n";
    let (_dir, home) = home_with(text);
    assert_eq!(rewrite_ports(&home, "http://127.0.0.1:4200").unwrap(), 1);
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        text.replace("127.0.0.1:4100", "127.0.0.1:4200")
    );
}
