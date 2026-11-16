//! The startup config steps as one write ([`super`]): each step's exact output bytes, at most one write, and
//! nothing changed by a refused or failed write.
//!
//! PREDICT: the write-state cases need the shared draft ([`super::Draft`], [`super::prepare`]) and do not compile
//! without it; the byte-identical cases hold for any implementation that runs the six steps in launch order.
//!
//! Fixtures (`fixtures/startup_config`): for each `<name>.in.toml`, `<name>.<mode>.out.toml` is the exact file after a
//! launch in that mode (`product`; `product-marked`, the registry migration already recorded; `dev`). A missing
//! mode file means the same bytes as the product output, and a missing product output means the input is left as it
//! is. `fresh.<mode>.out.toml` is the file a launch without a config writes.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};

use super::{Draft, Written, prepare, write_config};

const MARKER: &str = ".vw-registry-skills-default-v1";
const MODES: [&str; 3] = ["product", "product-marked", "dev"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/startup_config")
}

/// The fixtures spell absolute paths the Unix way. On Windows an absolute path needs a drive, so the same fixtures
/// get one there: the product keeps absolute extension paths and drops relative ones on every OS, and the test checks
/// exactly that everywhere.
fn os_paths(bytes: Vec<u8>) -> Vec<u8> {
    if !cfg!(windows) {
        return bytes;
    }
    String::from_utf8(bytes)
        .unwrap()
        .replace("\"/opt/tools\"", "\"C:/opt/tools\"")
        .replace("\"/abs/agents\"", "\"C:/abs/agents\"")
        .into_bytes()
}

fn expected(name: &str, mode: &str, input: Option<&[u8]>) -> Vec<u8> {
    for file in [
        format!("{name}.{mode}.out.toml"),
        format!("{name}.product.out.toml"),
    ] {
        if let Ok(bytes) = std::fs::read(fixtures().join(file)) {
            return os_paths(bytes);
        }
    }
    input.expect("an input or a product output").to_vec()
}

/// A launch in `mode` over `input` (`None`: no config) in a fresh home: the file after it, the number of writes,
/// and whether the registry migration is recorded.
fn launch(mode: &str, input: Option<&[u8]>) -> (Vec<u8>, usize, bool) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    if let Some(input) = input {
        std::fs::write(home.join("config.toml"), input).unwrap();
    }
    if mode == "product-marked" {
        std::fs::write(home.join(MARKER), b"1\n").unwrap();
    }
    let writes = Cell::new(0);
    let counted = |path: &Path, bytes: &[u8], replace: bool| {
        writes.set(writes.get() + 1);
        write_config(path, bytes, replace)
    };
    prepare(&home, mode != "dev", counted).unwrap();
    let after = std::fs::read(home.join("config.toml")).unwrap();
    (after, writes.get(), home.join(MARKER).exists())
}

#[test]
fn every_step_writes_the_same_bytes_in_one_write() {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".in.toml").map(str::to_owned)
        })
        .collect();
    names.sort();
    assert!(names.len() >= 16, "{names:?}");
    for name in &names {
        let input = os_paths(std::fs::read(fixtures().join(format!("{name}.in.toml"))).unwrap());
        for mode in MODES {
            let want = expected(name, mode, Some(&input));
            let (after, writes, marker) = launch(mode, Some(&input));
            assert!(
                after == want,
                "{name} ({mode}) differs:\n{}",
                String::from_utf8_lossy(&after)
            );
            assert_eq!(writes, usize::from(want != input), "{name} ({mode}) writes");
            // Invalid TOML skips the experiment step, so its migration stays to do.
            let recorded = mode == "product-marked" || name != "invalid";
            assert_eq!(marker, recorded, "{name} ({mode}) migration marker");
        }
    }
}

#[test]
fn a_launch_without_a_config_writes_the_whole_first_config_once() {
    for mode in MODES {
        let (after, writes, marker) = launch(mode, None);
        assert!(after == expected("fresh", mode, None), "fresh ({mode})");
        assert_eq!(writes, 1, "fresh ({mode})");
        assert!(marker, "fresh ({mode})");
    }
}

/// A config every step edits (relative extension path, old Large 4 values, no title model, no credit line, no trust
/// store key, an experiment on).
fn needs_every_step() -> Vec<u8> {
    std::fs::read(fixtures().join("user_all.in.toml")).unwrap()
}

#[test]
fn a_failing_combined_write_changes_nothing() {
    for product in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        let original = needs_every_step();
        std::fs::write(home.join("config.toml"), &original).unwrap();
        let failed = prepare(&home, product, |_, _, _| Err(io::Error::other("disk full")));
        assert_eq!(std::fs::read(home.join("config.toml")).unwrap(), original);
        assert!(
            !home.join(MARKER).exists(),
            "migration recorded without the write"
        );
        // The Large 4 step changed the text, so only the product launch fails.
        assert_eq!(failed.is_err(), product, "product launch: {product}");
    }
}

#[test]
fn a_failing_write_without_a_large_four_change_only_logs() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    let original = std::fs::read(fixtures().join("title_model_append.in.toml")).unwrap();
    std::fs::write(home.join("config.toml"), &original).unwrap();
    prepare(&home, true, |_, _, _| Err(io::Error::other("disk full"))).unwrap();
    assert_eq!(std::fs::read(home.join("config.toml")).unwrap(), original);
}

#[test]
fn a_config_changed_meanwhile_is_not_written() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    let config = home.join("config.toml");
    std::fs::write(&config, needs_every_step()).unwrap();
    let mut draft = Draft::open(&home).unwrap();
    let staged = draft.apply(
        |text, _| crate::title_model::edit(text),
        crate::title_model::TitleModel::Skipped,
    );
    assert!(staged.changed);
    let theirs = "active_model = \"theirs\"\n";
    std::fs::write(&config, theirs).unwrap();
    assert_eq!(draft.commit(), Written::Refused(super::CHANGED.to_owned()));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), theirs);
}

#[test]
fn a_config_that_appears_meanwhile_is_not_replaced() {
    for product in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        let config = home.join("config.toml");
        let theirs = "active_model = \"theirs\"\n";
        // The file appears after the read; the creating write must not replace it.
        let appeared = |path: &Path, bytes: &[u8], replace: bool| {
            assert!(!replace);
            std::fs::write(path, theirs).unwrap();
            write_config(path, bytes, replace)
        };
        prepare(&home, product, appeared).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), theirs);
        assert!(!home.join(MARKER).exists());
    }
}

#[test]
fn a_draft_reports_each_write_state() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "active_model = \"m\"\n").unwrap();
    let draft = Draft::open(&home).unwrap();
    assert_eq!(draft.commit(), Written::Unchanged);
    std::fs::create_dir(root.path().join("dir-home")).unwrap();
    std::fs::create_dir_all(root.path().join("dir-home/config.toml")).unwrap();
    let mut draft = Draft::open(&root.path().join("dir-home")).unwrap();
    let staged = draft.apply(|_, _| (Some("x = 1\n".to_owned()), ()), |_| ());
    assert!(!staged.changed, "a step edited a non-file");
    assert_eq!(
        draft.commit(),
        Written::Refused("config.toml is not a file".to_owned())
    );
}
