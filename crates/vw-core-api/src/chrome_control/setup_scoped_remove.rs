//! Removes one receipted native registration. Shared helper/Windows manifest survive until the last browser.
//! Uses setup's checked file operations and registration lock; foreign registrations are never removed.
//! Depends on: setup file operations and the registration lock.
//! Called by: per-browser setup removal.
use super::{
    ChromeControlConfig, HOST_NAME, Made, Path, RECEIPT, files, helper_name, home_of, manifest_for,
    manifest_path, places, problem, read_receipt, registration, registration_lock, registry_delete,
    removal, remove, targets, write_file,
};

pub(in crate::chrome_control) fn has_other(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
) -> bool {
    read_receipt(config, state).is_some_and(|receipt| {
        receipt.browsers.iter().any(|name| {
            name != browser && super::selection::selected(config, state, &receipt, name)
        })
    })
}

pub(in crate::chrome_control) fn remove_browser(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
    keep_shared: bool,
) -> Result<(), String> {
    super::migrate_selections(config, state)?;
    if !keep_shared && !has_other(config, state, browser) {
        return remove(config, state);
    }
    let _held = registration_lock();
    let result = remove_registration(config, state, browser);
    problem::finish(config, state, browser, result)
}

pub(super) fn remove_registration(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
) -> Result<(), String> {
    let mut receipt = read_receipt(config, state).ok_or("Browser setup is missing")?;
    let home = home_of(config)?;
    let target = targets(&home, config)
        .into_iter()
        .find(|target| target.name == browser)
        .or_else(|| {
            (browser == "Microsoft Edge").then(|| {
                let user_data = places::edge_data_for(&home, config);
                places::Target {
                    name: "Microsoft Edge",
                    hosts: (!cfg!(windows)).then(|| user_data.join("NativeMessagingHosts")),
                    user_data,
                    registry: cfg!(windows).then_some(config.edge_registry_root.as_str()),
                }
            })
        })
        .ok_or("Browser setup is unavailable")?;
    let manifest = target.hosts.map_or_else(
        || manifest_path(state, browser),
        |hosts| hosts.join(format!("{HOST_NAME}.json")),
    );
    // Chromium's Windows registry keys share one manifest. Remove only the chosen browser's key.
    let shared_manifest = cfg!(windows)
        && receipt
            .browsers
            .iter()
            .any(|name| name != browser && manifest_path(state, name) == manifest);
    if !shared_manifest && receipt.files.contains(&manifest) {
        if let Some(bytes) = files::read(config, &manifest)
            .map_err(|error| removal::permission_failure(config, state, &manifest, &error))?
        {
            registration::owned_manifest(
                &bytes,
                &manifest_for(&state.join("bin").join(helper_name()), config, browser),
            )?;
            files::remove(config, &manifest, &bytes)
                .map_err(|error| removal::permission_failure(config, state, &manifest, &error))?;
        }
        receipt.files.retain(|path| path != &manifest);
    }
    let root = if browser == "Mozilla Firefox" {
        &config.firefox_registry_root
    } else if browser == "Microsoft Edge" {
        &config.edge_registry_root
    } else {
        &config.windows_registry_root
    };
    let key = format!(r"{root}\{HOST_NAME}");
    if receipt.registry_keys.contains(&key) {
        registry_delete(&key, &manifest)?;
        receipt.registry_keys.retain(|saved| saved != &key);
    }
    let removed_data = receipt.data_for(browser);
    receipt
        .user_data
        .retain(|path| !removed_data.contains(path));
    receipt.browsers.retain(|name| name != browser);
    if let Some(selected) = &mut receipt.selected_browsers {
        selected.retain(|name| name != browser);
    }
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
    write_file(config, &mut Made::default(), &state.join(RECEIPT), &bytes)
}
