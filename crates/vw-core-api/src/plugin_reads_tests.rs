//! Read-boundary regressions for `market_discovery` and `plugin_identity`. Uses the pinned Python and
//! disposable trees; run by vw-core-api's tests without a window or server.

use std::path::PathBuf;
use std::time::Duration;

use super::{discovery_command, inventory_output};
use tokio::process::Command;

/// The pinned Vibe's Python: `VIBE_PYTHON` when set (the public CI job sets it, and a public checkout has no
/// versioned runtime), else the repository's pinned versioned runtime (`Scripts/python.exe` on Windows).
fn python() -> PathBuf {
    if let Some(python) = std::env::var_os("VIBE_PYTHON") {
        return PathBuf::from(python);
    }
    let venv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(format!(
            "vibe-{}",
            include_str!("../../../tools/public/codegen/vibe.pin").trim()
        ))
        .join("venv");
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// The pinned Vibe's server executable beside [`python`]. It must be the real file: on Windows the server's Python is
/// read from the executable itself (a uv/pip launcher or a script in `Scripts`), so a made-up path resolves to none.
fn server() -> PathBuf {
    python().with_file_name(format!("vibe-app-server{}", std::env::consts::EXE_SUFFIX))
}

fn command(script: &str) -> Command {
    let mut command = Command::new(python());
    command.args(["-I", "-B", "-c", script]).kill_on_drop(true);
    command
}

struct Scratch(PathBuf);

impl Scratch {
    /// Below the resolved temporary folder on Unix: Vibe reports resolved plugin paths, and the macOS temporary
    /// folder lies below the `/var` link. Windows keeps the folder as given (resolving adds the `\\?\` prefix).
    fn new(label: &str) -> Self {
        let base = std::env::temp_dir();
        let base = if cfg!(unix) {
            base.canonicalize().unwrap()
        } else {
            base
        };
        let path = base.join(format!(
            "vw-{label}-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pinned_plugin_read_regressions() {
    let output = std::process::Command::new(python())
        .args(["-I", "-B"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/public/python/test_plugin_reads.py"
        ))
        .output()
        .expect("pinned Vibe's Python must be installed to run plugin read regressions");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn oversized_inventory_output_is_refused() {
    let mut command = command("import sys; sys.stdout.write(' ' * (4 * 1024 * 1024 + 1))");
    let result = inventory_output(&mut command, Duration::from_secs(5)).await;
    assert!(result.is_err_and(|e| e.contains("too large")));
}

#[tokio::test]
async fn inventory_pipe_is_drained_while_running() {
    let mut command = command("import sys; sys.stdout.write(' ' * (256 * 1024) + '{}')");
    let output = inventory_output(&mut command, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap(),
        serde_json::json!({})
    );
}

#[tokio::test]
async fn inventory_deadline_includes_an_endless_output_stream() {
    let mut command = command("import os\nwhile True: os.write(1, b'x' * 8192)");
    let started = std::time::Instant::now();
    let result = inventory_output(&mut command, Duration::from_millis(150)).await;
    assert!(result.is_err_and(|e| e.contains("timed out")));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn inventory_deadline_waits_for_both_output_and_process_exit() {
    for script in [
        "import time; print('{}', flush=True); time.sleep(30)",
        "import os, time; os.close(1); time.sleep(30)",
    ] {
        let mut command = command(script);
        let started = std::time::Instant::now();
        let result = inventory_output(&mut command, Duration::from_millis(150)).await;
        assert!(result.is_err_and(|e| e.contains("timed out")));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn native_identity_cannot_bypass_the_tree_and_compatibility_limits() {
    let scratch = Scratch::new("native-read");
    let folder = &scratch.0;
    std::fs::create_dir_all(folder.join(".claude-plugin")).unwrap();
    std::fs::write(folder.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"candidate"}"#).unwrap();
    let identity = crate::plugin_identity::PluginIdentity::new(Some(python()));
    let bad = folder.join(".claude-plugin/plugin.json");
    std::fs::File::create(&bad)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    let name = identity.name(folder, folder).unwrap();
    assert_eq!(
        name, None,
        "native identity skipped the compatibility preflight"
    );
}

#[test]
fn over_budget_plugin_cannot_inherit_approval_at_launch() {
    let scratch = Scratch::new("plugin-launch");
    let home = scratch.0.join("home");
    let folder = home.join("plugins/candidate");
    std::fs::create_dir_all(folder.join(".claude-plugin")).unwrap();
    std::fs::write(folder.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"candidate"}"#).unwrap();
    std::fs::File::create(folder.join(".claude-plugin/plugin.json"))
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    let approvals = crate::plugin_approvals::PluginApprovals::new(&home).unwrap();
    approvals.set(&folder, Some("candidate")).unwrap();
    approvals.before_launch(&server()).unwrap();
    assert!(!folder.exists());
    assert!(
        home.join("plugins-off/candidate/.claude-plugin/plugin.json")
            .exists()
    );
    assert!(!home.join("plugin-data").exists());
}

/// The Tools reader reads, of the folders in `plugins/`, only those its sweep kept: a folder copied in after the
/// sweep is not read (and so not listed) until a sweep has checked it.
#[tokio::test]
async fn the_reader_skips_a_folder_copied_in_after_its_sweep() {
    let scratch = Scratch::new("plugin-late");
    let home = scratch.0.join("home");
    let plugin = |name: &str| {
        let folder = home.join("plugins").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.json"), format!(r#"{{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"{name}"}}"#)).unwrap();
        folder
    };
    let approvals = crate::plugin_approvals::PluginApprovals::new(&home).unwrap();
    approvals.before_launch(&server()).unwrap();
    approvals.set(&plugin("kept"), Some("kept")).unwrap();
    let kept = approvals.sweep_kept().unwrap();
    assert_eq!(kept, ["kept"]);
    plugin("late");
    let cwd = scratch.0.join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let options = serde_json::json!({"builtinRoots": [], "workspaceRoots": [], "kept": kept});
    let mut command = discovery_command(&python(), &home, &cwd, &options);
    let output = inventory_output(&mut command, Duration::from_secs(30))
        .await
        .unwrap();
    let view: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let listed = |name: &str| {
        view["plugins"].as_array().unwrap().iter().any(|row| {
            row["path"]
                .as_str()
                .is_some_and(|p| std::path::Path::new(p) == home.join("plugins").join(name))
        })
    };
    assert!(listed("kept"), "{view}");
    assert!(!listed("late"), "{view}");
    assert!(home.join("plugins/late").exists());
}

/// The Tools reader gives each plugin's version and author from its manifest, so cards can tell plugins apart.
#[tokio::test]
async fn the_reader_reports_each_plugins_version_and_author() {
    let scratch = Scratch::new("plugin-meta");
    let home = scratch.0.join("home");
    let folder = home.join("plugins/maker");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"maker","version":"1.4.2","author":{"name":"Example Maker"}}"#).unwrap();
    let cwd = scratch.0.join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let options = serde_json::json!({"builtinRoots": [], "workspaceRoots": [], "kept": ["maker"]});
    let mut command = discovery_command(&python(), &home, &cwd, &options);
    let output = inventory_output(&mut command, Duration::from_secs(30))
        .await
        .unwrap();
    let view: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let row = view["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "maker")
        .unwrap_or_else(|| panic!("{view}"));
    assert_eq!(row["version"], "1.4.2", "{row}");
    assert_eq!(row["author"], "Example Maker", "{row}");
}

/// Timing, run by hand: the sweep before each Vibe load checks every approved plugin's manifest name with one
/// pinned-Python process each. Prints the sweep time for 1, 10 and 30 approved plugins.
#[test]
#[ignore = "manual timing"]
fn sweep_time_with_many_approved_plugins() {
    for count in [1usize, 10, 30] {
        let scratch = Scratch::new("plugin-sweep-time");
        let home = scratch.0.join("home");
        let approvals = crate::plugin_approvals::PluginApprovals::new(&home).unwrap();
        approvals.before_launch(&server()).unwrap();
        for i in 0..count {
            let name = format!("plugin-{i}");
            let folder = home.join("plugins").join(&name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("plugin.json"),
                format!(r#"{{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"{name}"}}"#),
            )
            .unwrap();
            approvals.set(&folder, Some(&name)).unwrap();
        }
        let started = std::time::Instant::now();
        let kept = approvals.sweep_kept().unwrap();
        let took = started.elapsed();
        assert_eq!(kept.len(), count);
        println!(
            "sweep with {count} approved plugins: {} ms ({} ms each)",
            took.as_millis(),
            took.as_millis() / count as u128
        );
    }
}
