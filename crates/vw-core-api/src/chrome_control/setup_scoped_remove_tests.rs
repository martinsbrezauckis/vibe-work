//! Real receipt/file/registry removal in disposable homes. Both browser orders retain the shared helper until last.
use super::*;

#[test]
fn connecting_one_browser_then_removing_it_cleans_all_owned_files_and_keys() {
    for browser in ["Google Chrome", "Microsoft Edge"] {
        let scratch = Scratch::new(if browser == "Google Chrome" {
            "only-chrome"
        } else {
            "only-edge"
        });
        let config = scratch.config();
        let state = scratch.state();
        install_for(&config, &state, &scratch.vibe_home(), browser).unwrap();
        let receipt = read_receipt(&config, &state).unwrap();
        let keep_shared = scoped_removal::has_other(&config, &state, browser);
        scoped_removal::remove_browser(&config, &state, browser, keep_shared).unwrap();
        assert!(
            !state.exists(),
            "only {browser} was selected, but removal retained state; receipt: {receipt:?}"
        );
        for file in receipt.files {
            assert!(
                !file.exists(),
                "remaining helper/manifest: {}",
                file.display()
            );
        }
        #[cfg(windows)]
        for key in receipt.registry_keys {
            assert_eq!(registration(&key).0, Some(1), "remaining key: {key}");
        }
        assert_eq!(receipt.browsers, [browser]);
    }
}

#[test]
fn scoped_remove_preserves_other_registration_and_shared_files_in_both_orders() {
    for removed in ["Google Chrome", "Microsoft Edge"] {
        let scratch = Scratch::new(if removed == "Google Chrome" {
            "scoped-chrome"
        } else {
            "scoped-edge"
        });
        let config = scratch.config();
        let state = scratch.state();
        install(&config, &state, &scratch.vibe_home()).unwrap();
        install_for(&config, &state, &scratch.vibe_home(), "Microsoft Edge").unwrap();
        let kept = if removed == "Google Chrome" {
            "Microsoft Edge"
        } else {
            "Google Chrome"
        };
        let before = read_receipt(&config, &state).unwrap();
        std::fs::write(state.join("paired-profiles.json"), b"signed pairing store").unwrap();
        scoped_removal::remove_browser(&config, &state, removed, true).unwrap();
        let after = read_receipt(&config, &state).unwrap();
        assert_eq!(after.browsers, [kept]);
        assert_eq!(after.user_data, before.data_for(kept));
        assert_eq!(
            std::fs::read(state.join("paired-profiles.json")).unwrap(),
            b"signed pairing store"
        );
        assert!(state.join("bin").join(helper_name()).is_file());
        for target in targets(&scratch.root.join("home"), &config) {
            let path = target.hosts.map_or_else(
                || manifest_path(&state, target.name),
                |hosts| hosts.join(format!("{HOST_NAME}.json")),
            );
            // Only the two browsers installed above have manifests; Firefox (a default feature) was never set up.
            // Windows keeps both installed browsers' manifest files and removes the registry key instead.
            let installed = target.name == removed || target.name == kept;
            assert_eq!(
                path.exists(),
                installed && (cfg!(windows) || target.name == kept),
                "{}",
                target.name
            );
            #[cfg(windows)]
            if let Some(root) = target.registry {
                assert_eq!(
                    registration(&format!(r"{root}\{HOST_NAME}")).0,
                    Some(i32::from(target.name != kept))
                );
            }
        }
        scoped_removal::remove_browser(&config, &state, kept, false).unwrap();
        assert!(
            !state.exists(),
            "last removal cleans shared helper and records"
        );
        for file in before.files {
            assert!(
                !file.exists(),
                "remaining helper/manifest: {}",
                file.display()
            );
        }
        #[cfg(windows)]
        for key in before.registry_keys {
            assert_eq!(registration(&key).0, Some(1), "remaining key: {key}");
        }
    }
}

#[test]
fn reconnecting_a_removed_browser_preserves_the_other_browsers_receipt_and_pairing_store() {
    let scratch = Scratch::new("scoped-reconnect");
    let config = scratch.config();
    let state = scratch.state();
    install(&config, &state, &scratch.vibe_home()).unwrap();
    install_for(&config, &state, &scratch.vibe_home(), "Microsoft Edge").unwrap();
    scoped_removal::remove_browser(&config, &state, "Google Chrome", true).unwrap();
    let edge = read_receipt(&config, &state)
        .unwrap()
        .data_for("Microsoft Edge");
    std::fs::write(state.join("paired-profiles.json"), b"kept signed pins").unwrap();
    install_for(&config, &state, &scratch.vibe_home(), "Google Chrome").unwrap();
    let after = read_receipt(&config, &state).unwrap();
    assert_eq!(after.data_for("Microsoft Edge"), edge);
    assert!(after.browsers.iter().any(|name| name == "Google Chrome"));
    assert_eq!(
        std::fs::read(state.join("paired-profiles.json")).unwrap(),
        b"kept signed pins"
    );
}

#[cfg(unix)]
#[test]
fn scoped_remove_refuses_a_foreign_manifest_without_touching_the_other_browser() {
    let scratch = Scratch::new("scoped-foreign-host");
    let config = scratch.config();
    let state = scratch.state();
    install(&config, &state, &scratch.vibe_home()).unwrap();
    install_for(&config, &state, &scratch.vibe_home(), "Microsoft Edge").unwrap();
    let manifest = targets(&scratch.root.join("home"), &config)
        .remove(0)
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    let before = std::fs::read(state.join(RECEIPT)).unwrap();
    std::fs::write(&manifest, b"another installation").unwrap();
    assert!(scoped_removal::remove_browser(&config, &state, "Google Chrome", true).is_err());
    assert_eq!(std::fs::read(manifest).unwrap(), b"another installation");
    assert_eq!(std::fs::read(state.join(RECEIPT)).unwrap(), before);
    assert!(state.join("bin").join(helper_name()).is_file());
}
