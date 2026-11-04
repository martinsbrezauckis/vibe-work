//! Removes only configured registrations still owned by this setup. Used by Remove; foreign files/keys
//! and linked ancestors survive, and a failure retains the receipt so cleanup can be retried.
use super::{
    ChromeControlConfig, HOST_NAME, Path, files, helper_name, home_of, host_manifest,
    manifest_path, read_receipt, targets,
};

pub(crate) fn remove(config: &ChromeControlConfig, state: &Path) -> Result<(), String> {
    let _held = super::registration_lock();
    let receipt = read_receipt(state).unwrap_or_default();
    let home = home_of(config)?;
    let helper = state.join("bin").join(helper_name());
    let expected = host_manifest(&helper, &config.extension_id);
    let mut allowed: Vec<_> = targets(&home, config)
        .into_iter()
        .map(|target| {
            target.hosts.map_or_else(
                || manifest_path(state, target.name),
                |hosts| hosts.join(format!("{HOST_NAME}.json")),
            )
        })
        .collect();
    // Removal also recognizes Edge installed before policy disabled its setup target.
    allowed.push(
        super::places::edge_data(&home)
            .join("NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json")),
    );
    allowed.push(manifest_path(state, "Mozilla Firefox"));
    let mut files_to_remove = Vec::new();
    for path in receipt.files.iter().filter(|p| **p != helper) {
        if !allowed.contains(path) {
            return Err("Browser helper registration is not owned by this setup".into());
        }
        if let Some(bytes) = files::read(path).map_err(|e| e.to_string())? {
            super::registration::owned_manifest(&bytes, &expected)?;
            files_to_remove.push((path, bytes));
        }
    }
    for key in &receipt.registry_keys {
        let browser = [
            (&config.windows_registry_root, "Google Chrome"),
            (&config.edge_registry_root, "Microsoft Edge"),
            (&config.firefox_registry_root, "Mozilla Firefox"),
        ]
        .into_iter()
        .find_map(|(root, browser)| (key == &format!(r"{root}\{HOST_NAME}")).then_some(browser))
        .ok_or("Browser helper registration is not owned by this setup")?;
        super::registry_delete(key, &manifest_path(state, browser))?;
    }
    for (path, bytes) in files_to_remove {
        files::remove(path, &bytes).map_err(|e| e.to_string())?;
    }
    let parent = state.parent().ok_or("Browser setup has no parent folder")?;
    vw_platform::remove_owned_tree(parent, state).map_err(|e| e.to_string())
}
