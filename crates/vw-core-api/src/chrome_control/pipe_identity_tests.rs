//! Replaces a checked helper at the launch boundary; native recorders never use a real owner key.
//! Called by Pipe's unit tests on every OS, with a test-only callback after hashing.
use super::*;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};

fn recorder(root: &Path, name: &str) -> PathBuf {
    let source = root.join(format!("{name}.rs"));
    std::fs::write(&source, format!(r#"
use std::io::BufRead;
fn main() {{
    let dir = std::env::args_os().nth(3).unwrap();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).unwrap();
    if !line.is_empty() {{ std::fs::write(std::path::Path::new(&dir).join("{name}.received"), line).unwrap(); }}
}}
"#)).unwrap();
    let binary = root.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        std::process::Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success()
    );
    binary
}

#[tokio::test]
async fn replacement_after_check_never_receives_owner_line() {
    checked_launch(false).await;
}

#[tokio::test]
async fn overwrite_after_check_never_receives_owner_line() {
    checked_launch(true).await;
}

async fn checked_launch(overwrite: bool) {
    let root =
        std::env::temp_dir().join(format!("vw-helper-swap-{}-{overwrite}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let helper = root.join("bin").join(super::super::setup::helper_name());
    std::fs::rename(recorder(&root, "trusted"), &helper).unwrap();
    let replacement = recorder(&root, "swapped");
    let bytes = std::fs::read(&helper).unwrap();
    let mut config = super::super::setup::ChromeControlConfig {
        package: Some(root.clone()),
        helper_sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    {
        assert!(
            std::process::Command::new("codesign")
                .args(["--force", "--sign", "-", "--options", "kill"])
                .arg(&helper)
                .status()
                .unwrap()
                .success()
        );
        let output = std::process::Command::new("codesign")
            .args(["-d", "--verbose=4"])
            .arg(&helper)
            .output()
            .unwrap();
        config.helper_cdhash = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("CDHash=").map(str::to_owned));
        assert!(config.helper_cdhash.is_some());
        config.helper_sha256 = Some(format!(
            "{:x}",
            Sha256::digest(std::fs::read(&helper).unwrap())
        ));
    }
    let bytes = std::fs::read(&helper).unwrap();
    let secret = vw_vault::SecretString::new("disposable-owner-scalar".to_owned());
    assert!(
        Pipe::launch(&config, &root, blake3::hash(&bytes), &secret, Weak::new())
            .await
            .is_err()
    );
    assert!(
        root.join("trusted.received").exists(),
        "the pinned helper must receive stdin"
    );
    std::fs::remove_file(root.join("trusted.received")).unwrap();
    config.after_helper_check = Some(Arc::new(move |path| {
        if overwrite {
            let written = std::fs::write(path, std::fs::read(&replacement).unwrap());
            #[cfg(windows)]
            assert!(written.is_err());
            #[cfg(not(windows))]
            written.unwrap();
            return;
        }
        let renamed = std::fs::rename(path, path.with_extension("checked"));
        if renamed.is_ok() {
            std::fs::rename(&replacement, path).unwrap();
        }
        #[cfg(windows)]
        {
            assert!(renamed.is_err(), "checked executable must deny replacement");
            assert!(std::fs::OpenOptions::new().write(true).open(path).is_err());
            assert!(std::fs::remove_file(path).is_err());
        }
        #[cfg(not(windows))]
        assert!(renamed.is_ok());
    }));
    let result = Pipe::launch(&config, &root, blake3::hash(&bytes), &secret, Weak::new()).await;
    assert!(result.is_err(), "recorders do not implement MCP");
    assert!(
        !root.join("swapped.received").exists(),
        "replacement received the first stdin line"
    );
    #[cfg(not(target_os = "macos"))]
    assert!(root.join("trusted.received").exists());
    std::fs::remove_dir_all(root).unwrap();
}
