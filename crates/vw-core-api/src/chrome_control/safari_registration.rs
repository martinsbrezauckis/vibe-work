//! Registration of the embedded Safari extension only. Uses `PlugInKit`'s exact bundle path, never a bundle-wide purge.
//! Safari setup and removal call this on macOS; Linux tests verify arguments, failure and moved bundle paths.
use std::path::{Path, PathBuf};

fn extension_at(app: &Path) -> PathBuf {
    app.join("Contents/PlugIns/Your Safari Extension.appex")
}

pub(super) async fn change_at(program: &Path, app: &Path, remove: bool) -> Result<(), String> {
    use std::{process::Stdio, time::Duration};
    let extension = extension_at(app);
    if !extension.is_dir() {
        return Err(format!(
            "{} is not installed.",
            vw_vocabulary::texts::SAFARI_NAME
        ));
    }
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(program)
            .arg(if remove { "-r" } else { "-a" })
            .arg(extension)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    if matches!(status, Ok(Ok(status)) if status.success()) {
        return Ok(());
    }
    Err(if remove {
        "Safari could not be removed from Vibe Work."
    } else {
        "Safari could not be registered."
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_unregisters_only_the_embedded_extension_and_setup_registers_that_same_path() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("Moved Vibe Work.app");
        std::fs::create_dir_all(extension_at(&app)).unwrap();
        let program = root.path().join("pluginkit");
        // A fork in another test can inherit this process's open write descriptor and make exec
        // fail with ETXTBSY. Install from a separate process so this process never writes the program.
        let write_program = |text: &str| {
            let source = root.path().join("pluginkit.source");
            std::fs::write(&source, text).unwrap();
            assert!(
                std::process::Command::new("install")
                    .args(["-m", "0700"])
                    .arg(&source)
                    .arg(&program)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        write_program("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"${0%/*}/called\"\n");
        for (remove, flag) in [(true, "-r"), (false, "-a")] {
            change_at(&program, &app, remove).await.unwrap();
            let args = std::fs::read_to_string(root.path().join("called")).unwrap();
            assert_eq!(args, format!("{flag}\n{}\n", extension_at(&app).display()));
        }
        write_program("#!/bin/sh\nexit 1\n");
        assert!(
            change_at(&program, &app, true)
                .await
                .unwrap_err()
                .contains("could not be removed")
        );
        assert!(
            change_at(&root.path().join("missing"), &app, false)
                .await
                .is_err()
        );
        assert!(
            extension_at(&app).exists(),
            "registration never deletes bundled files"
        );
    }
}
