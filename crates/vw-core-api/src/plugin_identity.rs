//! Manifest identities for approval checks. Called by plugin approvals; uses the native JSON identity or
//! the same pinned Vibe resolver as Tools for compatibility formats and encoded manifests. No servers start here.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::error::{CoreError, ErrorCode};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

// Missing/encoded/foreign manifests still use Vibe's format precedence. An unreadable, linked,
// special or oversized native manifest cannot establish identity and must not reach the resolver.
fn native_manifest(folder: &Path) -> io::Result<Option<serde_json::Value>> {
    let path = folder.join("plugin.json");
    let metadata = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    let check = |metadata: &std::fs::Metadata| {
        if !metadata.file_type().is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            Err(io::Error::other(
                "the manifest is not a bounded regular file",
            ))
        } else {
            Ok(())
        }
    };
    check(&metadata)?;
    let file = std::fs::File::open(path)?;
    check(&file.metadata()?)?;
    let mut bytes = Vec::new();
    (&file).take(MAX_MANIFEST_BYTES).read_to_end(&mut bytes)?;
    check(&file.metadata()?)?;
    Ok(serde_json::from_slice(&bytes).ok())
}

pub(crate) struct PluginIdentity {
    python: Option<PathBuf>,
    runtime_python: Mutex<Option<PathBuf>>,
}

fn error(problem: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("The plugin's manifest name could not be checked: {problem}"),
    )
}

impl PluginIdentity {
    pub(crate) fn new(python: Option<PathBuf>) -> Self {
        Self {
            python,
            runtime_python: Mutex::new(None),
        }
    }

    /// The actual server path also covers a deferred install or a newly selected Vibe environment.
    pub(crate) fn set_program(&self, program: &Path) {
        *self
            .runtime_python
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = program.parent().map(|bin| {
            bin.join(if cfg!(windows) {
                "python.exe"
            } else {
                "python"
            })
        });
    }

    pub(crate) fn name(&self, folder: &Path, home: &Path) -> Result<Option<String>, CoreError> {
        // Marketplace manifests and ordinary native manifests need only their literal name. All other cases
        // use Vibe, whose format precedence, text decoding and package-name normalization are authoritative.
        let Ok(native) = native_manifest(folder) else {
            return Ok(None);
        };
        if let Some(manifest) = native
            && manifest["$schema"] == "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
            && let Some(name) = manifest["name"].as_str()
        {
            return Ok(Some(name.to_owned()));
        }
        let python = self
            .python
            .clone()
            .or_else(|| {
                self.runtime_python
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .ok_or_else(|| error("Vibe's Python environment is unavailable"))?;
        let mut command = Command::new(python);
        command
            .args(["-I", "-B", "-c", include_str!("plugin_identity.py")])
            .arg(folder)
            .env_clear()
            .env("VIBE_HOME", home)
            .current_dir(home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for key in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "HOME", "USERPROFILE"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(error)?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("no resolver output"));
        };
        let reader = std::thread::spawn(move || {
            let mut stdout = stdout;
            let mut bytes = Vec::new();
            (&mut stdout)
                .take(MAX_MANIFEST_BYTES + 1)
                .read_to_end(&mut bytes)?;
            // Keep draining even excess output so the child can exit without filling its pipe.
            io::copy(&mut stdout, &mut io::sink())?;
            if bytes.len() as u64 > MAX_MANIFEST_BYTES {
                return Err(io::Error::other("the resolver output is too large"));
            }
            Ok(bytes)
        });
        let until = Instant::now() + Duration::from_secs(30);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
                problem => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error(match problem {
                        Err(problem) => problem.to_string(),
                        _ => "Vibe's plugin resolver timed out".to_owned(),
                    }));
                }
            }
        };
        let output = reader
            .join()
            .map_err(|_| error("the resolver output reader stopped"))?;
        if !status?.success() {
            return Err(error("Vibe's plugin resolver failed"));
        }
        let names: Vec<String> = serde_json::from_slice(&output.map_err(error)?).map_err(error)?;
        Ok((names.len() == 1).then(|| names[0].clone()))
    }
}
