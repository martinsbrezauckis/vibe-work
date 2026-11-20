use crate::core::Shared;
use serde_json::{Value, json};
pub(super) fn prepare_name(shared: &Shared, chat: &str, request: &Value) -> Result<String, String> {
    let name = match request["args"]["filename"].as_str() {
        Some(name) => {
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            if stem.is_empty() {
                return Err("The recording filename is invalid".into());
            }
            format!("{stem}.webm")
        }
        None => format!("browser-{}.webm", super::page_files::token()?),
    };
    if !valid_filename(&name) {
        return Err("The recording filename is invalid".into());
    }
    let folder = super::output_file::folder(shared, chat)?;
    if folder.metadata(&name).is_ok() {
        return Err("A file with this recording name already exists in the chat folder".into());
    }
    Ok(name)
}
pub(super) fn valid_filename(name: &str) -> bool {
    if name.len() > 255
        || name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
    {
        return false;
    }
    let reserved = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&reserved.as_str()) {
        return false;
    }
    for prefix in ["COM", "LPT"] {
        if let Some(suffix) = reserved.strip_prefix(prefix)
            && ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&suffix)
        {
            return false;
        }
    }
    true
}
pub(super) fn save(shared: &Shared, chat: &str, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let folder = super::output_file::folder(shared, chat)?;
    super::output_file::write(&folder, name, bytes)
        .map_err(|_| "The WebM could not be saved in this chat folder")?;
    if folder
        .metadata(name)
        .map_err(|_| "The saved WebM could not be read back")?
        .len()
        != bytes.len() as u64
    {
        return Err("The saved WebM size did not match".into());
    }
    Ok(json!({"path":name,"bytes":bytes.len(),"mime":"video/webm"}))
}
