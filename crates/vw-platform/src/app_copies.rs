//! Registered copies of this containing application, read from Launch Services for Safari setup diagnostics.
//! Uses the running bundle's identifier, without scanning disks, registering or removing applications.
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSBundle;
use std::path::PathBuf;

/// Other candidates are filtered against the running bundle and embedded assets by the core.
#[must_use]
pub fn registered() -> Vec<PathBuf> {
    let Some(identifier) = NSBundle::mainBundle().bundleIdentifier() else {
        return Vec::new();
    };
    NSWorkspace::sharedWorkspace()
        .URLsForApplicationsWithBundleIdentifier(&identifier)
        .iter()
        .filter(|url| url.isFileURL())
        .filter_map(|url| url.path().map(|path| PathBuf::from(path.to_string())))
        .collect()
}
