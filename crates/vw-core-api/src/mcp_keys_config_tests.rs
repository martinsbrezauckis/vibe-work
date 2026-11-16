//! The config edits on the forms Vibe itself writes.
//!
//! PREDICT (written before the first run): Vibe's own writer saves an empty server list inline (`mcp_servers = []`);
//! `add_entry` and `add_imported` accept only `[[mcp_servers]]` tables, so both refuse that file as `Unsupported`
//! (a key server added in Tools or brought by Import is then never saved). A non-empty inline list stays refused:
//! rewriting it would change the person's other entries.

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
    add_imported(&home, "docs", &server).unwrap();
    let list = servers_in(&home);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"].as_str(), Some("docs"));
}

#[test]
fn a_non_empty_inline_server_list_is_left_alone() {
    let text = "active_model = \"m\"\nmcp_servers = [{ name = \"a\", transport = \"stdio\", command = \"a\" }]\n\n[[models]]\nname = \"m\"\nalias = \"m\"\n";
    let (_dir, home) = home_with(text);
    assert!(matches!(
        add_entry(&home, "echokey", "http://127.0.0.1:4100/vw-mcp/x"),
        Err(ConfigEditError::Unsupported)
    ));
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        text
    );
}
