//! Saves composer attachments inside the session's project folder. Depends on cap-std, vw-platform and the
//! session cwd supplied by `documents`; called before a file is exposed as model context.
//! Directory handles confine writes, existing links are refused, and names are never overwritten.

use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};

use cap_std::fs::{Dir, OpenOptions};
use vw_platform::canonical_plain;

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let clean: String = last
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(120)
        .collect();
    let clean = clean.trim_matches([' ', '.']);
    if clean.is_empty() {
        "file".to_owned()
    } else {
        clean.to_owned()
    }
}

fn child_dir(parent: &Dir, name: &str) -> Result<Dir, String> {
    match parent.symlink_metadata(name) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err("attachment folder is a link or is not a directory".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use cap_std::fs::DirBuilderExt as _;
                let mut builder = cap_std::fs::DirBuilder::new();
                builder.mode(0o700);
                parent
                    .create_dir_with(name, &builder)
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(not(unix))]
            parent.create_dir(name).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    parent.open_dir(name).map_err(|e| e.to_string())
}

fn session_dir(cwd: &Path, session_id: &str) -> Result<(PathBuf, Dir), String> {
    if !safe_id(session_id) {
        return Err("invalid attachment session id".to_owned());
    }
    let root = canonical_plain(cwd).map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("session folder is not a directory".to_owned());
    }
    let project =
        Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(|e| e.to_string())?;
    let attachments = child_dir(&project, "attachments")?;
    let session = child_dir(&attachments, session_id)?;
    Ok((root, session))
}

fn saved_path(root: &Path, session_id: &str, leaf: &str) -> Result<PathBuf, String> {
    let path = root.join("attachments").join(session_id).join(leaf);
    let resolved = canonical_plain(&path).map_err(|e| e.to_string())?;
    if !resolved.starts_with(root) {
        return Err("attachment resolved outside the session folder".to_owned());
    }
    Ok(resolved)
}

/// Saves one bounded, already validated attachment beneath `cwd/attachments/session_id`.
/// The returned path is absolute and can be named in the model prompt. A collision uses a
/// numbered name; no existing file is replaced, including one planted by another process.
///
/// # Errors
/// An invalid identifier, unsafe folder, I/O failure, or exhausted collision suffixes.
pub(crate) fn save(
    cwd: &Path,
    session_id: &str,
    document_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    if !safe_id(document_id) {
        return Err("invalid attachment document id".to_owned());
    }
    let (root, session) = session_dir(cwd, session_id)?;
    let base = format!("{document_id}-{}", safe_name(name));
    for suffix in 0..1000 {
        let leaf = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let mut file = match session.open_with(&leaf, &private_options()) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = session.remove_file(&leaf);
            return Err(error.to_string());
        }
        let resolved = saved_path(&root, session_id, &leaf);
        if resolved.is_err() {
            let _ = session.remove_file(&leaf);
        }
        return resolved;
    }
    Err("too many attachments with the same name".to_owned())
}

/// Saves a picture at one deterministic name so a retried send reuses the original file.
/// Existing regular files are read through the confined directory handle and reused only when
/// their contents exactly match. A mismatch or link is refused rather than renamed or replaced.
///
/// # Errors
/// Invalid identifiers, unsafe folders, I/O failures, or conflicting existing bytes.
pub(crate) fn save_image(
    cwd: &Path,
    session_id: &str,
    image_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    if !safe_id(image_id) || bytes.len() > 10 * 1024 * 1024 {
        return Err("invalid attachment image id or size".to_owned());
    }
    let (root, session) = session_dir(cwd, session_id)?;
    let leaf = format!("{image_id}-{}", safe_name(name));
    let created = match session.open_with(&leaf, &private_options()) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                let _ = session.remove_file(&leaf);
                return Err(error.to_string());
            }
            true
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let meta = session.symlink_metadata(&leaf).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != bytes.len() as u64
            {
                return Err("an attachment with this image id already exists".to_owned());
            }
            let mut existing = Vec::with_capacity(bytes.len());
            session
                .open(&leaf)
                .map_err(|e| e.to_string())?
                .take((10 * 1024 * 1024 + 1) as u64)
                .read_to_end(&mut existing)
                .map_err(|e| e.to_string())?;
            if existing != bytes {
                return Err("an attachment with this image id has different bytes".to_owned());
            }
            false
        }
        Err(error) => return Err(error.to_string()),
    };
    let path = saved_path(&root, session_id, &leaf);
    if created && path.is_err() {
        let _ = session.remove_file(&leaf);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project() -> PathBuf {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "vw-attachment-test-{}-{:016x}",
            std::process::id(),
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn saves_under_project_and_never_overwrites_a_collision() {
        let project = temp_project();
        let root = canonical_plain(&project).unwrap();
        let first = save(&project, "session-1", "doc-1", "../same.txt", b"first").unwrap();
        let second = save(&project, "session-1", "doc-1", "same.txt", b"second").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(root.join("attachments/session-1")));
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(first.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert!(save(&project, "../escape", "doc-2", "x.txt", b"x").is_err());
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn image_retry_reuses_identical_file_and_refuses_conflict() {
        let project = temp_project();
        let root = canonical_plain(&project).unwrap();
        let first = save_image(&project, "session-1", "img-abc", "photo.png", b"first").unwrap();
        let again = save_image(&project, "session-1", "img-abc", "photo.png", b"first").unwrap();
        assert_eq!(first, again);
        assert!(first.starts_with(root.join("attachments/session-1")));
        assert!(save_image(&project, "session-1", "img-abc", "photo.png", b"other").is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        std::fs::remove_dir_all(project).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_attachment_directory_symlink() {
        let project = temp_project();
        let outside = temp_project();
        std::os::unix::fs::symlink(&outside, project.join("attachments")).unwrap();
        assert!(save(&project, "session-1", "doc-1", "file.txt", b"x").is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_dir_all(project).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn image_retry_rejects_symlink_leaf() {
        let project = temp_project();
        let outside = temp_project();
        let folder = project.join("attachments/session-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(outside.join("photo.png"), b"same").unwrap();
        std::os::unix::fs::symlink(outside.join("photo.png"), folder.join("img-abc-photo.png"))
            .unwrap();
        assert!(save_image(&project, "session-1", "img-abc", "photo.png", b"same").is_err());
        assert_eq!(std::fs::read(outside.join("photo.png")).unwrap(), b"same");
        std::fs::remove_dir_all(project).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
