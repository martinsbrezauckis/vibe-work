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
const CATALOG_MARKER: &str = ".vw-model-catalog-default-v1";
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
    let mut mismatches = Vec::new();
    for name in &names {
        let input = os_paths(std::fs::read(fixtures().join(format!("{name}.in.toml"))).unwrap());
        for mode in MODES {
            let want = expected(name, mode, Some(&input));
            let (after, writes, marker) = launch(mode, Some(&input));
            if after == want {
                assert_eq!(writes, usize::from(want != input), "{name} ({mode}) writes");
            } else {
                mismatches.push(format!(
                    "@@@ {name}.{mode} BEGIN\n{}@@@ END\n",
                    String::from_utf8_lossy(&after)
                ));
            }
            // Invalid TOML skips the experiment step, so its migration stays to do.
            let recorded = mode == "product-marked" || name != "invalid";
            assert_eq!(marker, recorded, "{name} ({mode}) migration marker");
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join(""));
}

#[test]
fn a_launch_without_a_config_writes_the_whole_first_config_once() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        let (after, writes, marker) = launch(mode, None);
        if after != expected("fresh", mode, None) {
            mismatches.push(format!(
                "@@@ fresh.{mode} BEGIN\n{}@@@ END\n",
                String::from_utf8_lossy(&after)
            ));
        }
        assert_eq!(writes, 1, "fresh ({mode})");
        assert!(marker, "fresh ({mode})");
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join(""));
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
        assert!(
            !home.join(CATALOG_MARKER).exists(),
            "catalog default recorded without the write"
        );
        // The catalog migration is a non-critical startup edit.
        assert!(failed.is_ok(), "product launch: {product}");
    }
}

#[test]
fn a_failing_offline_model_write_stops_the_product_launch() {
    for product in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(CATALOG_MARKER), b"1\n").unwrap();
        let original = std::fs::read(fixtures().join("default_model.in.toml")).unwrap();
        std::fs::write(home.join("config.toml"), &original).unwrap();
        let failed = prepare(&home, product, |_, _, _| Err(io::Error::other("disk full")));
        assert_eq!(std::fs::read(home.join("config.toml")).unwrap(), original);
        assert_eq!(failed.is_err(), product, "product launch: {product}");
    }
}

#[test]
fn a_later_catalog_off_choice_survives_product_and_dev_launches() {
    for product in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(CATALOG_MARKER), b"1\n").unwrap();
        std::fs::write(
            home.join("config.toml"),
            std::fs::read(fixtures().join("default_model.in.toml")).unwrap(),
        )
        .unwrap();
        prepare(&home, product, write_config).unwrap();
        let config: toml::Table = std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(config["enable_model_catalog"].as_bool(), Some(false));
        assert!(config.contains_key("models"), "offline local model remains");
    }
}

#[test]
fn a_saved_314_catalog_choice_migrates_once_without_changing_the_model() {
    for product in [true, false] {
        for catalog_on in [true, false] {
            for marker in [true, false] {
                let root = tempfile::tempdir().unwrap();
                let home = root.path().join("vibe-home");
                std::fs::create_dir_all(&home).unwrap();
                if marker {
                    std::fs::write(home.join(CATALOG_MARKER), b"1\n").unwrap();
                }
                // This is a complete 3.1.4-era config, including a saved catalog choice and Large 4.
                let old = std::fs::read_to_string(fixtures().join("settled.in.toml"))
                    .unwrap()
                    .replace(
                        "experimental_enable_model_catalog = false",
                        if catalog_on {
                            "experimental_enable_model_catalog = true"
                        } else {
                            "experimental_enable_model_catalog = false"
                        },
                    );
                std::fs::write(home.join("config.toml"), old).unwrap();
                prepare(&home, product, write_config).unwrap();
                let first = std::fs::read_to_string(home.join("config.toml")).unwrap();
                let config: toml::Table = first.parse().unwrap();
                assert!(!config.contains_key("experimental_enable_model_catalog"));
                assert_eq!(config["enable_model_catalog"].as_bool(), Some(catalog_on));
                assert_eq!(config["active_model"].as_str(), Some("large-4"));
                assert_eq!(config.contains_key("models"), marker || !catalog_on);
                prepare(&home, product, write_config).unwrap();
                assert_eq!(
                    std::fs::read_to_string(home.join("config.toml")).unwrap(),
                    first
                );
            }
        }
    }
}

// A saved catalog-on choice removes an exact app seed on its first startup in either mode.
#[test]
fn catalog_upgrade_retires_the_exact_app_model_in_product_and_dev() {
    for product in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        let catalog_on = std::fs::read_to_string(fixtures().join("settled.in.toml"))
            .unwrap()
            .replace(
                "experimental_enable_model_catalog = false",
                "experimental_enable_model_catalog = true",
            );
        std::fs::write(home.join("config.toml"), catalog_on).unwrap();
        prepare(&home, product, write_config).unwrap();
        let config: toml::Table = std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(config["enable_model_catalog"].as_bool(), Some(true));
        assert!(!config.contains_key("models"), "product={product}");
        assert_eq!(config["active_model"].as_str(), Some("large-4"));
    }
}

// The first catalog migration removes the old app row; later launches respect the marker.
#[test]
fn catalog_seed_cleanup_runs_once_after_the_marker() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    let old = std::fs::read_to_string(fixtures().join("settled.in.toml"))
        .unwrap()
        .replace(
            "experimental_enable_model_catalog = false",
            "experimental_enable_model_catalog = true",
        );
    std::fs::write(home.join("config.toml"), &old).unwrap();
    prepare(&home, true, write_config).unwrap();
    assert!(home.join(CATALOG_MARKER).is_file());
    let first: toml::Table = std::fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(!first.contains_key("models"));

    std::fs::write(home.join("config.toml"), old).unwrap();
    prepare(&home, true, write_config).unwrap();
    let second: toml::Table = std::fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(second["active_model"].as_str(), Some("large-4"));
    assert!(second.contains_key("models"));
}

#[test]
fn a_failing_write_without_a_large_four_change_only_logs() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    let original = needs_every_step();
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
        |text, _| crate::title_model::edit(text, true),
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
        assert!(!home.join(CATALOG_MARKER).exists());
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
