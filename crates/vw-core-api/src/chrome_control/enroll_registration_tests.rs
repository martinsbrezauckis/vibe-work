//! Settings and Debug API Connect with receipted foreign registrations, using the enrollment rig.
//! PREDICT: Connect calls passive registration repair and returns the old ownership refusal for stale and live
//! foreign entries; a successful replacement must return its one-time hint on both Chrome and Edge commands.
use super::*;
use crate::CommandResult;
use std::path::{Path, PathBuf};

fn leave_foreign(rig: &Rig, edge: bool, file: &Path) -> PathBuf {
    let config = &rig.core.shared.chrome_control.config;
    let host = "com.vibework.chrome_control";
    if cfg!(windows) {
        let root = if edge {
            &config.edge_registry_root
        } else {
            &config.windows_registry_root
        };
        let key = format!(r"{root}\{host}");
        assert!(
            std::process::Command::new("reg")
                .args(["add", &key, "/ve", "/d", file.to_str().unwrap(), "/f"])
                .status()
                .unwrap()
                .success()
        );
        PathBuf::from(key)
    } else {
        let browser = if edge {
            "microsoft-edge"
        } else {
            "google-chrome"
        };
        let path = config
            .user_home
            .as_ref()
            .unwrap()
            .join(".config")
            .join(browser)
            .join("NativeMessagingHosts")
            .join(format!("{host}.json"));
        std::fs::copy(file, &path).unwrap();
        path
    }
}

fn contents(path: &Path) -> Vec<u8> {
    if cfg!(windows) {
        extension_registry::host_value(path.to_str().unwrap(), &extension_registry::Native)
            .unwrap()
            .unwrap()
            .into_bytes()
    } else {
        std::fs::read(path).unwrap()
    }
}

async fn check(live: bool) {
    for origin in [Origin::Ui, Origin::DebugApi] {
        for edge in [false, true]
            .into_iter()
            .filter(|edge| !edge || cfg!(feature = "your-edge"))
        {
            let rig = Rig::new(
                &format!("connect-registration-{live}-{origin:?}-{edge}"),
                ticket_answer,
            )
            .await;
            let chrome = &rig.core.shared.chrome_control;
            let state = chrome.dir().unwrap();
            let browser = if edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            };
            setup::install_for(
                &chrome.config,
                state,
                &rig.core.shared.instructions.own_home,
                browser,
            )
            .unwrap();
            let file = chrome.config.package.as_ref().unwrap().join("foreign.json");
            let helper = file.with_file_name("foreign-helper");
            if live {
                std::fs::write(&helper, b"foreign").unwrap();
            }
            std::fs::write(
                &file,
                json!({"name":"com.vibework.chrome_control", "path":helper}).to_string(),
            )
            .unwrap();
            let path = leave_foreign(&rig, edge, &file);
            let before = contents(&path);
            let request = || {
                if edge {
                    ChromeControlRequest::OpenEdgeStore {}
                } else {
                    ChromeControlRequest::Connect {}
                }
            };
            let result = rig.core.chrome_control(origin, request()).await;
            if live {
                let error = result.unwrap_err().message;
                let shown = if cfg!(windows) { &file } else { &helper };
                assert!(
                    error.contains(&shown.display().to_string()),
                    "{browser}, {origin:?}: {error}"
                );
                assert!(error.ends_with("then retry Connect"), "{error}");
                assert_eq!(contents(&path), before);
            } else {
                let result =
                    result.unwrap_or_else(|error| panic!("{browser}, {origin:?}: {error:?}"));
                let CommandResult::ChromeControl {
                    hint: Some(hint), ..
                } = result
                else {
                    panic!("replacement notice missing: {result:?}");
                };
                assert!(hint.contains(&helper.display().to_string()), "{hint}");
                assert_ne!(contents(&path), before);
                let result = rig.core.chrome_control(origin, request()).await.unwrap();
                assert!(matches!(
                    result,
                    CommandResult::ChromeControl { hint: None, .. }
                ));
            }
            rig.core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn connect_commands_replace_receipted_stale_registration_and_return_notice_once() {
    check(false).await;
}

#[tokio::test]
async fn connect_commands_name_and_preserve_receipted_live_foreign_registration() {
    check(true).await;
}
