//! Immutable browser collection files under the authenticated chat folder. No arbitrary paths or overwrite.
//! Directory handles constrain reads/writes; append creates a new artifact and retains its previous collection.
use super::Collection;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::io::{Read, Write};
const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn previous(directory: &Dir, name: &str) -> Result<Collection, String> {
    let id = name
        .strip_prefix("browser-data-")
        .and_then(|s| s.strip_suffix(".json"));
    if id.is_none_or(|s| s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err("Invalid appendFrom collection; use the collection filename returned by the previous export.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|_| "Previous collection is unavailable; export again without appendFrom.")?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= MAX_BYTES)
    {
        return Err("Previous collection is too large or not a file; start a new export.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Previous collection cannot be read; start a new export.")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Previous collection exceeds the limit; start a new export.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Previous collection is malformed; start a new export.".into())
}

fn write(directory: &Dir, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = directory.open_with(name, &options)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        // Read back the same directory entry before returning its filename.
        let mut read_options = OpenOptions::new();
        read_options.read(true).follow(FollowSymlinks::No);
        let mut actual = Vec::new();
        directory
            .open_with(name, &read_options)?
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut actual)?;
        if actual != bytes {
            return Err(std::io::Error::other("export readback differs"));
        }
        Ok(())
    })();
    drop(file);
    if result.is_err() {
        let _ = directory.remove_file(name);
    }
    result
}

pub(super) fn save(
    directory: &Dir,
    collection: &Collection,
    format: &str,
) -> Result<(String, String, usize), String> {
    let metadata = serde_json::to_vec(collection)
        .map_err(|_| "Collection encoding failed; retry the export.")?;
    if metadata.len() as u64 > MAX_BYTES {
        return Err("The collection exceeds 8 MiB; start a separate export.".into());
    }
    let bytes = if format == "csv" {
        super::encode::csv(&collection.columns, &collection.rows)
    } else {
        super::encode::xlsx(&collection.columns, &collection.rows)?
    };
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "Export filename unavailable; retry the export.")?;
    let base = format!("browser-data-{:032x}", u128::from_be_bytes(random));
    let data_name = format!("{base}.json");
    let name = format!("{base}.{format}");
    let failure = "Export could not be written and verified in the chat folder; choose a writable folder and retry.";
    write(directory, &data_name, &metadata).map_err(|_| failure)?;
    if write(directory, &name, &bytes).is_err() {
        let _ = directory.remove_file(&data_name);
        return Err(failure.into());
    }
    Ok((name, data_name, bytes.len()))
}
