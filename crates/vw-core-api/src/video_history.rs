//! Optional bounded ownership index for saved video images. Reuses `document_save`'s chat attachments;
//! Files reads these exact bytes after restart without opening the source or loading a decoder.
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

const SESSION_BYTES: usize = 32 * 1024 * 1024;
const TOTAL_BYTES: usize = 128 * 1024 * 1024;
const MAX_RECORDS: usize = 512;
const INDEX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) session: String,
    pub(super) source: String,
    pub(super) call: String,
    pub(super) cwd: PathBuf,
    pub(super) id: String,
    pub(super) digest: String,
    pub(super) bytes: usize,
    pub(super) metadata: Value,
}
impl Record {
    fn relative(&self) -> String {
        format!("attachments/{}/video-{}-frames.png", self.session, self.id)
    }
    fn remove(&self) -> Result<(), String> {
        if self.bytes == 0 {
            return Ok(());
        }
        // No-follow directory handles: a replaced parent can never redirect eviction into another folder.
        let dir = match crate::search_walk::root(&self.cwd.join("attachments").join(&self.session))
        {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        match dir.remove_file(format!("video-{}-frames.png", self.id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub(super) fn read(&self) -> Result<Vec<u8>, String> {
        let file = crate::search_walk::open(&self.cwd, &self.relative())
            .map_err(|_| "Saved video frames are unavailable.")?;
        if file
            .metadata()
            .map_err(|_| "Saved video frames are unavailable.")?
            .len()
            != self.bytes as u64
        {
            return Err("Saved video frames have changed.".into());
        }
        let mut bytes = Vec::new();
        file.take(vw_preview::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Saved video frames are unavailable.")?;
        if bytes.len() != self.bytes || blake3::hash(&bytes).to_hex().as_str() != self.digest {
            return Err("Saved video frames have changed.".into());
        }
        Ok(bytes)
    }
}
#[derive(Default)]
struct State {
    records: Vec<Record>,
    deleted: BTreeSet<String>,
}
pub(crate) struct History {
    path: PathBuf,
    state: Mutex<State>,
}
fn safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
impl History {
    pub(crate) fn load(path: PathBuf) -> Result<Self, String> {
        let records: Vec<Record> = match std::fs::File::open(&path) {
            Ok(f) => {
                let mut bytes = Vec::new();
                f.take(INDEX_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > INDEX_BYTES {
                    return Err("Video index exceeds its limit.".into());
                }
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        if records.len() > MAX_RECORDS
            || records
                .iter()
                .any(|r| !safe(&r.id) || !safe(&r.session) || r.bytes > vw_preview::MAX_BYTES)
            || records.iter().map(|r| r.bytes).sum::<usize>() > TOTAL_BYTES
        {
            return Err("Invalid video index.".into());
        }
        Ok(Self {
            path,
            state: Mutex::new(State {
                records,
                deleted: BTreeSet::new(),
            }),
        })
    }
    fn write(&self, records: &[Record]) -> Result<(), String> {
        let bytes = serde_json::to_vec(records).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > INDEX_BYTES {
            return Err("Video index exceeds its limit.".into());
        }
        vw_platform::replace_private(&self.path, &bytes).map_err(|e| e.to_string())
    }
    pub(crate) fn save(
        &self,
        cwd: &Path,
        session: &str,
        source: &str,
        call: &str,
        mut reply: Value,
    ) -> Result<(), String> {
        let id = reply["video_id"]
            .as_str()
            .filter(|s| safe(s))
            .ok_or("Invalid video identity.")?
            .to_owned();
        let encoded = reply[vw_agent_tools::wire::IMAGE_KEY]["data"]
            .as_str()
            .ok_or("Missing video image.")?;
        if encoded.len() > vw_preview::MAX_BYTES.div_ceil(3) * 4 {
            return Err("Video image exceeds its limit.".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid video image.")?;
        if !safe(session) || bytes.len() > vw_preview::MAX_BYTES {
            return Err("Invalid video image or session.".into());
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.deleted.contains(session) {
            return Err("This chat was deleted.".into());
        }
        if state.records.iter().any(|r| r.id == id) {
            return Err("Video identity already saved.".into());
        }
        reply
            .as_object_mut()
            .ok_or("Invalid video result.")?
            .remove(vw_agent_tools::wire::IMAGE_KEY);
        let cwd = vw_platform::canonical_plain(cwd).map_err(|e| e.to_string())?;
        let record = Record {
            session: session.into(),
            source: source.into(),
            call: call.into(),
            cwd,
            id,
            digest: blake3::hash(&bytes).to_hex().to_string(),
            bytes: bytes.len(),
            metadata: reply,
        };
        let mut next = state.records.clone();
        let mut removed = Vec::new();
        while next
            .iter()
            .filter(|r| r.session == session)
            .map(|r| r.bytes)
            .sum::<usize>()
            + bytes.len()
            > SESSION_BYTES
            || next.iter().filter(|r| r.session == session).count() >= 64
        {
            let at = next
                .iter()
                .position(|r| r.session == session)
                .ok_or("Video storage limit.")?;
            removed.push(next.remove(at));
        }
        while next.len() >= MAX_RECORDS
            || next.iter().map(|r| r.bytes).sum::<usize>() + bytes.len() > TOTAL_BYTES
        {
            removed.push(next.remove(0));
        }
        // Do not lose ownership of an image that could not be evicted, or exceed the byte cap.
        for old in &removed {
            old.remove()?;
        }
        crate::document_save::save_image(
            &record.cwd,
            session,
            &format!("video-{}", record.id),
            "frames.png",
            &bytes,
        )?;
        next.push(record.clone());
        if let Err(e) = self.write(&next) {
            let _ = record.remove();
            return Err(e);
        }
        state.records = next;
        Ok(())
    }
    pub(crate) fn outcome(
        &self,
        session: &str,
        source: &str,
        call: &str,
        metadata: Value,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.deleted.contains(session) {
            return Err("This chat was deleted.".into());
        }
        let mut next = state.records.clone();
        let removed = if next.len() >= MAX_RECORDS {
            Some(next.remove(0))
        } else {
            None
        };
        next.push(Record {
            session: session.into(),
            source: source.into(),
            call: call.into(),
            cwd: PathBuf::new(),
            id: vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?,
            digest: String::new(),
            bytes: 0,
            metadata,
        });
        if let Some(old) = removed {
            old.remove()?;
        }
        self.write(&next)?;
        state.records = next;
        Ok(())
    }
    pub(crate) fn read(
        &self,
        session: &str,
        handle: &str,
    ) -> Result<crate::files_view::FileContent, String> {
        let (id, cell) = handle
            .strip_prefix("video:")
            .and_then(|s| s.split_once(':'))
            .ok_or("Invalid video frame.")?;
        let cell = cell.parse::<usize>().map_err(|_| "Invalid video frame.")?;
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.deleted.contains(session) {
            return Err("This chat was deleted.".into());
        }
        let record = state
            .records
            .iter()
            .find(|r| r.session == session && r.id == id)
            .ok_or("Saved video frames are unavailable or were removed to make room.")?;
        crate::video_image::content(record, handle, cell)
    }
    pub(crate) fn overlay(&self, value: &mut Value) {
        if value["type"] == "effect"
            && let (Some(source), Some(call)) = (value["sessionId"].as_str(), value["id"].as_str())
        {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(record) = state
                .records
                .iter()
                .rev()
                .find(|r| r.source == source && r.call == call)
            {
                value["vwVideo"] = crate::video_image::projection(&record.metadata);
            }
        }
        match value {
            Value::Object(map) => map.values_mut().for_each(|v| self.overlay(v)),
            Value::Array(list) => list.iter_mut().for_each(|v| self.overlay(v)),
            _ => {}
        }
    }
    pub(crate) fn forget(&self, session: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.deleted.insert(session.into());
        let removed: Vec<_> = state
            .records
            .iter()
            .filter(|r| r.session == session)
            .cloned()
            .collect();
        let next: Vec<_> = state
            .records
            .iter()
            .filter(|r| r.session != session)
            .cloned()
            .collect();
        self.write(&next)?;
        state.records = next;
        // Revoke access durably even if an external move/link/permission change prevents cleanup.
        let mut error = None;
        for record in &removed {
            if let Err(reason) = record.remove() {
                error = Some(reason);
            }
        }
        error.map_or(Ok(()), Err)
    }
}
#[cfg(test)]
#[path = "video_history_tests.rs"]
mod tests;
