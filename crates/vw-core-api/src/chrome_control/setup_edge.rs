//! Adds a browser to an existing setup without replacing the shared running helper or owner identity.
//! Uses setup's checked pin and receipt; called after confirming the browser setup offer.
use super::{
    ChromeControlConfig, HOST_NAME, Made, Path, RECEIPT, absent_for, helper_name, home_of,
    manifest_for, manifest_path, packaged_helper, read_receipt, registry_set, targets, write_file,
};

pub(in crate::chrome_control) fn register_edge(
    config: &ChromeControlConfig,
    state: &Path,
) -> Result<(), String> {
    let result = register_browser(config, state, "Microsoft Edge");
    super::problem::finish(config, state, "Microsoft Edge", result)
}

pub(super) fn register_browser(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
) -> Result<(), String> {
    super::places::require_installed(config, browser)?;
    if browser == "Microsoft Edge" && !cfg!(feature = "your-edge") {
        return Err(absent_for(true).to_owned());
    }
    if browser == "Microsoft Edge"
        && let Some(reason) = super::super::edge_policy::refusal(config)
    {
        return Err(reason.to_owned());
    }
    let _held = super::registration_lock();
    super::selection::migrate(config, state)?;
    let mut receipt = read_receipt(config, state).ok_or("Browser setup is missing")?;
    if receipt.browsers.iter().any(|name| name == browser) {
        return super::registration::repair(config, state, Some(browser));
    }
    if !config.is_macos() {
        super::registration::repair(config, state, None)?;
    }
    let (_, pinned) = packaged_helper(config)?;
    let helper = state.join("bin").join(helper_name());
    if !super::files::matches(config, &helper, &pinned)
        .map_err(|e| super::problem::io_message(config, &e))?
    {
        return Err(vw_vocabulary::texts::CHROME_PACKAGE_CHANGED.to_owned());
    }
    let home = home_of(config)?;
    let target = targets(&home, config)
        .into_iter()
        .find(|t| t.name == browser)
        .ok_or("Browser setup is unavailable")?;
    let mut made = Made::default();
    let result = (|| {
        if let Some(hosts) = target.hosts {
            let path = hosts.join(format!("{HOST_NAME}.json"));
            super::registration::ensure_manifest(
                config,
                &path,
                &manifest_for(&helper, config, browser),
                false,
            )?;
            made.written
                .push((path.clone(), manifest_for(&helper, config, browser)));
            made.files.push(path);
        } else if let Some(root) = target.registry {
            let file = manifest_path(state, browser);
            let manifest = manifest_for(&helper, config, browser);
            let owned = receipt.files.contains(&file);
            super::registration::ensure_manifest(config, &file, &manifest, owned)?;
            if !owned {
                made.written.push((file.clone(), manifest));
                made.files.push(file.clone());
            }
            let key = format!(r"{root}\{HOST_NAME}");
            registry_set(&key, &file)?;
            made.keys.push((key.clone(), file));
            receipt.registry_keys.push(key);
        }
        receipt.files.extend(made.files.clone());
        receipt.browsers.push(target.name.into());
        receipt.selected_browsers = Some(receipt.browsers.clone());
        receipt.user_data.push(target.user_data);
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
        write_file(config, &mut made, &state.join(RECEIPT), &bytes)
    })();
    if result.is_err() {
        made.undo(config);
    }
    result
}
