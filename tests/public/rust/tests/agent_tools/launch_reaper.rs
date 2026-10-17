//! Cleanup for detached-agent fixtures, including a panic before the caller registers the returned pid.
//! Reads only this fixture's durable launches file; delegates tree termination to Reaper on drop.

use super::Reaper;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use vw_procmon::ProcessRef;

pub struct LaunchReaper {
    path: PathBuf,
    reaper: Reaper,
}

impl LaunchReaper {
    pub fn new(data: &Path) -> Self {
        Self {
            path: data.join("launches.json"),
            reaper: Reaper::default(),
        }
    }

    pub fn push(&self, pid: u32) {
        self.reaper.push(pid);
    }
}

#[derive(Deserialize)]
struct LaunchFile {
    launches: Vec<Launch>,
}

#[derive(Deserialize)]
struct Launch {
    process: ProcessRef,
}

impl Drop for LaunchReaper {
    fn drop(&mut self) {
        if let Ok(bytes) = std::fs::read(&self.path)
            && let Ok(file) = serde_json::from_slice::<LaunchFile>(&bytes)
        {
            for launch in file.launches {
                if vw_procmon::identity(launch.process.pid) == Some(launch.process) {
                    self.reaper.push(launch.process.pid);
                }
            }
        }
    }
}
