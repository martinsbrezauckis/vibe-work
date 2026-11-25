//! `retire_kept_medium` on files: only what the app wrote, proven by its marker, is removed.
#![allow(clippy::unwrap_used)]

use super::*;

const KEPT: &str = r#"

# Vibe Work: retain a model this person picked before Vibe's catalog replaced its built-in list.
[[models]]
name = "mistral-vibe-cli-latest"
provider = "mistral"
alias = "mistral-medium-3.5"
display_name = "Mistral Medium 3.5"
temperature = 1.0
input_price = 1.5
output_price = 7.5
cached_input_price = 0.15
thinking = "high"
thinking_levels = ["off", "high"]
supports_images = true
max_context_length = 262144
"#;
const OTHER: &str = "\n[[models]]\nname = \"my-model\"\nprovider = \"mine\"\nalias = \"mine\"\n";

struct Install {
    _root: tempfile::TempDir,
    home: std::path::PathBuf,
    sessions: std::path::PathBuf,
}

fn install(config: Option<&str>, marker: bool) -> Install {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe");
    let sessions = root.path().join("sessions");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    if let Some(config) = config {
        std::fs::write(home.join("config.toml"), config).unwrap();
    }
    if marker {
        std::fs::write(sessions.join(MARKER), "retained\n").unwrap();
    }
    Install {
        _root: root,
        home,
        sessions,
    }
}

impl Install {
    fn run(&self) {
        retire_kept_medium(&self.home, Some(&self.sessions));
    }
    fn config(&self) -> String {
        std::fs::read_to_string(self.home.join("config.toml")).unwrap()
    }
    fn marker(&self) -> bool {
        self.sessions.join(MARKER).exists()
    }
}

#[test]
fn the_row_written_by_an_earlier_version_goes_with_its_marker() {
    let base = "active_model = \"mistral-large-4-0\"\n";
    let it = install(Some(&format!("{base}{KEPT}")), true);
    it.run();
    assert_eq!(it.config(), base);
    assert!(!it.marker());
}

#[test]
fn other_rows_and_settings_around_it_are_kept_byte_for_byte() {
    let before = format!("active_model = \"x\"\n{OTHER}{KEPT}\n[mcp]\nkeep = true\n");
    let it = install(Some(&before), true);
    it.run();
    let after = it.config();
    assert!(!after.contains("mistral-medium-3.5"), "{after}");
    assert!(
        after.contains(OTHER) && after.contains("[mcp]\nkeep = true\n"),
        "{after}"
    );
    assert!(after.starts_with("active_model = \"x\"\n"), "{after}");
    assert!(!it.marker());
}

#[test]
fn without_the_marker_config_is_never_touched() {
    let before = format!("active_model = \"x\"\n{KEPT}");
    let it = install(Some(&before), false);
    it.run();
    assert_eq!(
        it.config(),
        before,
        "a row without the app's marker is the person's own"
    );
}

#[test]
fn a_row_with_other_facts_stays_and_the_marker_is_retired() {
    let edited = KEPT.replace("input_price = 1.5", "input_price = 9.9");
    let before = format!("active_model = \"x\"\n{edited}");
    let it = install(Some(&before), true);
    it.run();
    assert_eq!(it.config(), before);
    assert!(!it.marker());
}

#[test]
fn a_file_vibe_rewrote_without_our_comment_still_loses_the_row() {
    let plain = KEPT.replace(
        "# Vibe Work: retain a model this person picked before Vibe's catalog replaced its built-in list.\n",
        "",
    );
    let it = install(Some(&format!("active_model = \"x\"\n{plain}{OTHER}")), true);
    it.run();
    let after = it.config();
    assert!(!after.contains("mistral-medium-3.5"), "{after}");
    assert!(
        after.contains("my-model") && after.contains("active_model = \"x\""),
        "{after}"
    );
}

#[test]
fn an_unreadable_config_keeps_the_marker_for_the_next_start() {
    let it = install(Some("this is [not toml"), true);
    it.run();
    assert_eq!(it.config(), "this is [not toml");
    assert!(it.marker(), "retry at the next start");
}

#[test]
fn inline_rows_are_not_reshaped_and_the_marker_waits() {
    let inline = String::from(
        "models = [{name = \"mistral-vibe-cli-latest\", provider = \"mistral\", alias = \"mistral-medium-3.5\", display_name = \"Mistral Medium 3.5\", temperature = 1.0, input_price = 1.5, output_price = 7.5, cached_input_price = 0.15, thinking = \"high\", thinking_levels = [\"off\", \"high\"], supports_images = true, max_context_length = 262144}]\n",
    );
    let it = install(Some(&inline), true);
    it.run();
    assert_eq!(it.config(), inline);
    assert!(it.marker());
}

#[test]
fn a_missing_config_or_sessions_folder_is_harmless() {
    let it = install(None, true);
    it.run();
    assert!(!it.marker(), "nothing to remove, nothing to wait for");
    retire_kept_medium(&it.home, None);
}

#[test]
fn a_marker_that_is_not_a_plain_file_is_left_alone() {
    let before = format!("active_model = \"x\"\n{KEPT}");
    let it = install(Some(&before), false);
    std::fs::create_dir(it.sessions.join(MARKER)).unwrap();
    it.run();
    assert_eq!(it.config(), before);
    assert!(it.sessions.join(MARKER).is_dir());
}

#[test]
fn running_again_after_the_marker_is_gone_does_nothing() {
    let it = install(Some(&format!("active_model = \"x\"\n{KEPT}")), true);
    it.run();
    let once = it.config();
    it.run();
    assert_eq!(it.config(), once);
}
