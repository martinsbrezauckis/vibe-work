//! Resolves ambiguous old Chromium receipts from browser-local pairing history. Setup, repair and removal
//! call this under the registration lock; only registrations change, never pairing records or owner credentials.
use super::{ChromeControlConfig, Made, Path, RECEIPT, Receipt, files, read_receipt, write_file};

fn ambiguous(config: &ChromeControlConfig, receipt: &Receipt) -> bool {
    !config.is_macos()
        && ["Google Chrome", "Microsoft Edge"]
            .iter()
            .all(|browser| receipt.browsers.iter().any(|name| name == browser))
}

pub(super) fn selected(
    config: &ChromeControlConfig,
    state: &Path,
    receipt: &Receipt,
    browser: &str,
) -> bool {
    if let Some(selected) = &receipt.selected_browsers {
        return selected.iter().any(|name| name == browser);
    }
    !ambiguous(config, receipt)
        || !matches!(browser, "Google Chrome" | "Microsoft Edge")
        || paired(config, state, browser)
}

fn read_json(config: &ChromeControlConfig, path: &Path) -> Option<serde_json::Value> {
    let bytes = files::read(config, path).ok()??;
    serde_json::from_slice(&bytes).ok()
}

fn paired(config: &ChromeControlConfig, state: &Path, browser: &str) -> bool {
    let history = match browser {
        "Google Chrome" => state.join("paired.json"),
        "Microsoft Edge" => state.join("edge/paired.json"),
        _ => return false,
    };
    if read_json(config, &history).is_some_and(|value| value == true) {
        return true;
    }
    // Like paired.json, this local history is selection evidence only. The helper still authenticates
    // its signed pins with the owner key on admission; migration neither admits nor rewrites a pairing.
    let Some(envelope) = read_json(config, &state.join("paired-profiles.json")) else {
        return false;
    };
    if envelope["type"] != "signed"
        || !envelope["signature"]
            .as_str()
            .is_some_and(|sig| hex(sig, 128))
    {
        return false;
    }
    let Some(record) = envelope["payload"]
        .as_str()
        .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
    else {
        return false;
    };
    if record["message"]["kind"] != "paired-profiles/1" {
        return false;
    }
    record["message"]["profiles"]
        .as_object()
        .is_some_and(|profiles| {
            profiles.iter().any(|(profile, key)| {
                let id = if browser == "Microsoft Edge" {
                    profile.strip_prefix("edge:")
                } else {
                    profile.strip_prefix("chrome:").or(Some(profile.as_str()))
                };
                id.is_some_and(super::super::enroll::uuid_shape)
                    && key
                        .as_str()
                        .is_some_and(|key| key.starts_with("04") && hex(key, 130))
            })
        })
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Persist the decision before removing anything: a failed second removal must not turn the remaining
/// unselected browser into an apparently unambiguous single-browser receipt on the next start.
pub(super) fn migrate(config: &ChromeControlConfig, state: &Path) -> Result<(), String> {
    let Some(mut receipt) = read_receipt(config, state) else {
        return Ok(());
    };
    if receipt.selected_browsers.is_none() {
        if !ambiguous(config, &receipt) {
            return Ok(());
        }
        receipt.selected_browsers = Some(
            receipt
                .browsers
                .iter()
                .filter(|name| selected(config, state, &receipt, name))
                .cloned()
                .collect(),
        );
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
        write_file(config, &mut Made::default(), &state.join(RECEIPT), &bytes)?;
    }
    for browser in &receipt.browsers {
        if !selected(config, state, &receipt, browser) {
            super::scoped_removal::remove_registration(config, state, browser)?;
        }
    }
    Ok(())
}
