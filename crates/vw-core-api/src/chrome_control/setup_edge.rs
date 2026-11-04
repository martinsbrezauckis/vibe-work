//! Adds Edge to an existing Chrome setup without replacing the shared running helper or owner identity.
//! Uses setup's checked pin and receipt; called after confirming the browser setup offer.
use super::{
    ChromeControlConfig, HOST_NAME, Made, Path, RECEIPT, absent_for, helper_name, home_of,
    host_manifest, packaged_helper, read_receipt, registry_set, targets, write_file,
};

pub(in crate::chrome_control) fn register_edge(
    config: &ChromeControlConfig,
    state: &Path,
) -> Result<(), String> {
    if !cfg!(feature = "your-edge") {
        return Err(absent_for(true).to_owned());
    }
    if let Some(reason) = super::super::edge_policy::refusal(config) {
        return Err(reason.to_owned());
    }
    let mut receipt = read_receipt(state).ok_or("Browser setup is missing")?;
    if receipt
        .browsers
        .iter()
        .any(|browser| browser == "Microsoft Edge")
    {
        return super::repair_registrations(config, state, Some("Microsoft Edge"));
    }
    let _held = super::registration_lock();
    super::registration::repair(config, state, None)?;
    let (_, pinned) = packaged_helper(config)?;
    let helper = state.join("bin").join(helper_name());
    if std::fs::read(&helper).map_err(|e| e.to_string())? != pinned {
        return Err(vw_vocabulary::texts::CHROME_PACKAGE_CHANGED.to_owned());
    }
    let home = home_of(config)?;
    let target = targets(&home, config)
        .into_iter()
        .find(|t| t.name == "Microsoft Edge")
        .ok_or("Edge setup is unavailable")?;
    let mut made = Made::default();
    let result = (|| {
        if let Some(hosts) = target.hosts {
            let path = hosts.join(format!("{HOST_NAME}.json"));
            super::registration::ensure_manifest(
                &path,
                &host_manifest(&helper, &config.extension_id),
                false,
            )?;
            made.written
                .push((path.clone(), host_manifest(&helper, &config.extension_id)));
            made.files.push(path);
        } else if let Some(root) = target.registry {
            let file = state.join("host").join(format!("{HOST_NAME}.json"));
            let key = format!(r"{root}\{HOST_NAME}");
            registry_set(&key, &file)?;
            made.keys.push((key.clone(), file));
            receipt.registry_keys.push(key);
        }
        receipt.files.extend(made.files.clone());
        receipt.browsers.push(target.name.into());
        receipt.user_data.push(target.user_data);
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
        write_file(&mut made, &state.join(RECEIPT), &bytes)
    })();
    if result.is_err() {
        made.undo();
    }
    result
}
