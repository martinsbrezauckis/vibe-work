//! Installs the helper and selected native host. Uses checked setup I/O; macOS browser failures are isolated.
use super::{
    ChromeControlConfig, ChromeSetupView, HOST_NAME, Made, Path, RECEIPT, Receipt, edge, firefox,
    helper_name, home_of, manifest_for, manifest_path, packaged_helper, places, problem,
    read_receipt, registration, registration_lock, registry_set, targets, view_for, write_file,
};

/// Sets up Chrome Control (see the module docs). On error nothing stays changed.
#[cfg(test)]
pub(crate) fn install(
    config: &ChromeControlConfig,
    state: &Path,
    vibe_home: &Path,
) -> Result<ChromeSetupView, String> {
    install_for(config, state, vibe_home, "Google Chrome")
}

/// Registers only the selected browser so another card cannot retain unrequested shared ownership.
pub(in crate::chrome_control) fn install_for(
    config: &ChromeControlConfig,
    state: &Path,
    vibe_home: &Path,
    browser: &str,
) -> Result<ChromeSetupView, String> {
    places::require_installed(config, browser)?;
    let result = install_inner(config, state, vibe_home, browser);
    problem::finish(config, state, browser, result)
}

fn install_inner(
    config: &ChromeControlConfig,
    state: &Path,
    vibe_home: &Path,
    browser: &str,
) -> Result<ChromeSetupView, String> {
    if read_receipt(config, state).is_some_and(|receipt| {
        receipt
            .files
            .contains(&state.join("bin").join(helper_name()))
    }) {
        edge::register_browser(config, state, browser)?;
        problem::finish(config, state, browser, Ok(()))?;
        return Ok(view_for(config, Some(state), vibe_home, browser));
    }
    let held = registration_lock();
    let (_, bytes) = packaged_helper(config)?;
    let home = home_of(config)?;
    let mut made = Made::default();
    let result = (|| -> Result<Receipt, String> {
        for target in targets(&home, config)
            .into_iter()
            .filter(|t| t.name == browser)
        {
            if target.name == "Mozilla Firefox" {
                firefox::require_unregistered(&target)?;
            }
        }
        let helper = state.join("bin").join(helper_name());
        write_file(config, &mut made, &helper, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| problem::io_message(config, &e))?;
        }
        let mut receipt = Receipt::default();
        for target in targets(&home, config)
            .into_iter()
            .filter(|t| t.name == browser)
        {
            let manifest = manifest_for(&helper, config, target.name);
            match (&target.hosts, target.registry) {
                (Some(hosts), _) => {
                    let file = hosts.join(format!("{HOST_NAME}.json"));
                    registration::ensure_manifest(config, &file, &manifest, false)?;
                    made.written.push((file.clone(), manifest.clone()));
                    made.files.push(file);
                }
                (None, Some(root)) => {
                    let file = manifest_path(state, target.name);
                    if !made.files.contains(&file) {
                        registration::ensure_manifest(config, &file, &manifest, false)?;
                        made.written.push((file.clone(), manifest.clone()));
                        made.files.push(file.clone());
                    }
                    let key = format!(r"{root}\{HOST_NAME}");
                    registry_set(&key, &file)?;
                    made.keys.push((key.clone(), file.clone()));
                    receipt.registry_keys.push(key);
                }
                (None, None) => {}
            }
            receipt.browsers.push(target.name.to_owned());
            receipt.user_data.push(target.user_data);
        }
        receipt.selected_browsers = Some(receipt.browsers.clone());
        receipt.files.clone_from(&made.files);
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
        write_file(config, &mut made, &state.join(RECEIPT), &bytes)?;
        Ok(receipt)
    })();
    match result {
        Ok(_) => {
            drop(held);
            problem::finish(config, state, browser, Ok(()))?;
            Ok(view_for(config, Some(state), vibe_home, browser))
        }
        Err(problem) => {
            made.undo(config);
            Err(problem)
        }
    }
}
