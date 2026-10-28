//! Disposable file or null keyring for real Vibe fixtures, selected through keyring's normal config.
//! Called only by test harnesses; tokens stay under the same scratch root as HOME and VIBE_HOME.

use std::path::PathBuf;
use vw_supervisor::SupervisorConfig;

/// The keyring stand-in: a JSON file next to this module, inside the sandbox's writable run folder.
const KEYRING_BACKEND: &str = r#"
import json, os
from keyring.backend import KeyringBackend
PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "keyring.json")
class FileKeyring(KeyringBackend):
    priority = 1
    def _load(self):
        try:
            with open(PATH) as f:
                return json.load(f)
        except OSError:
            return {}
    def _save(self, data):
        with open(PATH, "w") as f:
            json.dump(data, f)
    def get_password(self, service, username):
        return self._load().get(service + "\0" + username)
    def set_password(self, service, username, password):
        data = self._load()
        data[service + "\0" + username] = password
        self._save(data)
    def delete_password(self, service, username):
        data = self._load()
        data.pop(service + "\0" + username, None)
        self._save(data)
"#;

/// Select the fixture backend explicitly, including hosts without a desktop secret-service bus.
pub fn install(config: &mut SupervisorConfig) -> PathBuf {
    let home = config.home.as_ref().expect("a disposable HOME");
    let backend = home.join("test-keyring");
    std::fs::create_dir_all(&backend).unwrap();
    std::fs::write(backend.join("vwtestkeyring.py"), KEYRING_BACKEND).unwrap();
    configure(
        config,
        &format!(
            "default-keyring=vwtestkeyring.FileKeyring\nkeyring-path={}\n",
            backend.display()
        ),
    );
    backend
}

/// Model-only launch probes need no tokens and keep backend selection out of the observed environment.
pub fn disable(config: &mut SupervisorConfig) {
    configure(config, "default-keyring=keyring.backends.null.Keyring\n");
}

fn configure(config: &mut SupervisorConfig, settings: &str) {
    let home = config.home.as_ref().expect("a disposable HOME");
    let xdg = home.join(".config");
    let rc = xdg.join(if cfg!(windows) {
        "Python Keyring"
    } else {
        "python_keyring"
    });
    std::fs::create_dir_all(&rc).unwrap();
    std::fs::write(rc.join("keyringrc.cfg"), format!("[backend]\n{settings}")).unwrap();
    // Let keyring read keyring-path from its normal configuration before importing the backend.
    config
        .extra_env
        .retain(|(name, _)| name != "PYTHON_KEYRING_BACKEND");
    config
        .extra_env
        .push(("XDG_CONFIG_HOME".into(), xdg.clone().into_os_string()));
    if cfg!(windows) {
        // Keyring uses LOCALAPPDATA on Windows, even when HOME and XDG_CONFIG_HOME are disposable.
        config
            .extra_env
            .push(("LOCALAPPDATA".into(), xdg.into_os_string()));
    }
}
