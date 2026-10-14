//! Filesystem rules for project instruction files.
//! Depends on cap-std directory handles, rustix on Unix, and the project instruction error helpers.
//! Called by the handler; no file operation uses a client-supplied absolute path.

use crate::error::{CoreError, ErrorCode};
use crate::instructions_view::{InstructionsDoc, InstructionsScope};
use crate::project_instructions::{invalid, io_error, refuse};
use cap_std::fs::{Dir, OpenOptions};
use std::fs;
use std::io::{Read, Write};
#[cfg(unix)]
use std::path::Component;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const LIMIT: usize = 256 * 1024;
static TMP_NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn check_name(name: &str) -> Result<&str, CoreError> {
    if name == "AGENTS.md" {
        return Ok(name);
    }
    let Some(file) = name.strip_prefix("docs/instructions/") else {
        return Err(invalid(
            "only AGENTS.md and direct docs/instructions/*.md files are allowed",
        ));
    };
    let Some(stem) = file.get(..file.len().saturating_sub(3)) else {
        return Err(invalid(
            "the instruction file name must be one direct Markdown file",
        ));
    };
    let upper = stem.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if file.len() > 96
        || !file.to_ascii_lowercase().ends_with(".md")
        || stem.is_empty()
        || !stem.as_bytes()[0].is_ascii_alphanumeric()
        || !stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || reserved
    {
        return Err(invalid(
            "the instruction file name must be one direct Markdown file",
        ));
    }
    Ok(name)
}

pub(crate) fn open_root(folder: &Path) -> Result<Dir, CoreError> {
    if !folder.is_absolute() {
        return Err(refuse(
            "folderUnavailable",
            "the project's saved main folder is unavailable",
        ));
    }
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, openat};
        let mut current = openat(
            rustix::fs::CWD,
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| io_error(&std::io::Error::from(e)))?;
        for component in folder.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    current = openat(
                        &current,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|_| {
                        refuse(
                            "folderUnavailable",
                            "the project's saved main folder is unavailable or linked",
                        )
                    })?;
                }
                _ => {
                    return Err(refuse(
                        "folderUnavailable",
                        "the project's saved main folder is not a direct absolute path",
                    ));
                }
            }
        }
        Ok(Dir::from_std_file(fs::File::from(current)))
    }
    #[cfg(not(unix))]
    {
        if fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(refuse(
                "folderUnavailable",
                "the project's saved main folder is linked",
            ));
        }
        Dir::open_ambient_dir(folder, cap_std::ambient_authority())
            .map_err(|error| io_error(&error))
    }
}

fn open_child(parent: &Dir, name: &str, create: bool) -> Result<Dir, CoreError> {
    let metadata = parent.symlink_metadata(name);
    match metadata {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(refuse(
                "linkedPath",
                "linked instruction folders cannot be opened here",
            ));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(refuse(
                "notEditable",
                "the instruction path is not a folder",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            if let Err(error) = parent.create_dir(name)
                && error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(io_error(&error));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "the instruction folder does not exist",
            ));
        }
        Err(error) => return Err(io_error(&error)),
    }
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, openat};
        let fd = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| {
            refuse(
                "linkedPath",
                "linked instruction folders cannot be opened here",
            )
        })?;
        Ok(Dir::from_std_file(fs::File::from(fd)))
    }
    #[cfg(not(unix))]
    {
        parent.open_dir(name).map_err(|error| io_error(&error))
    }
}

pub(crate) fn file_dir(
    folder: &Path,
    name: &str,
    create: bool,
) -> Result<(Dir, String), CoreError> {
    check_name(name)?;
    let root = open_root(folder)?;
    if name == "AGENTS.md" {
        return Ok((root, name.to_owned()));
    }
    let docs = open_child(&root, "docs", create)?;
    let instructions = open_child(&docs, "instructions", create)?;
    Ok((
        instructions,
        name.strip_prefix("docs/instructions/")
            .unwrap_or(name)
            .to_owned(),
    ))
}

fn read_file(dir: &Dir, name: &str) -> Result<Vec<u8>, CoreError> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, openat};
        let fd = openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| refuse("notEditable", "the instruction file changed or is linked"))?;
        fs::File::from(fd)
    };
    #[cfg(not(unix))]
    let file = dir.open(name).map_err(|error| io_error(&error))?;
    let meta = file.metadata().map_err(|error| io_error(&error))?;
    if !meta.is_file() || meta.len() > LIMIT as u64 {
        return Err(refuse(
            "notEditable",
            "the instruction file is not a regular file under 256 KiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(&error))?;
    if bytes.len() > LIMIT {
        return Err(refuse(
            "notEditable",
            "the instruction file is over 256 KiB",
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_doc(folder: &Path, name: &str) -> Result<InstructionsDoc, CoreError> {
    check_name(name)?;
    let path = folder.join(name);
    let mut doc = InstructionsDoc {
        scope: InstructionsScope::Project,
        path: path.to_string_lossy().into_owned(),
        folder: Some(folder.to_string_lossy().into_owned()),
        exists: false,
        text: String::new(),
        editable: true,
        note: None,
        cli_available: None,
    };
    let (dir, leaf) = match file_dir(folder, name, false) {
        Ok(found) => found,
        Err(error) if error.code == ErrorCode::NotFound => return Ok(doc),
        Err(error) => return Err(error),
    };
    let metadata = match dir.symlink_metadata(&leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(doc),
        Err(error) => return Err(io_error(&error)),
    };
    doc.exists = true;
    if metadata.file_type().is_symlink() {
        doc.editable = false;
        doc.note = Some("This file is a link and cannot be edited here.".to_owned());
    } else if !metadata.is_file() {
        doc.editable = false;
        doc.note = Some("This path is not a regular file.".to_owned());
    } else if metadata.len() > LIMIT as u64 {
        doc.editable = false;
        doc.note = Some("This file is over 256 KiB.".to_owned());
    } else {
        match read_file(&dir, &leaf).map(String::from_utf8) {
            Ok(Ok(text)) => doc.text = text,
            Ok(Err(_)) => {
                doc.editable = false;
                doc.note = Some("This file is not UTF-8 text.".to_owned());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(doc)
}

pub(crate) fn checked_doc(
    folder: &Path,
    name: &str,
    expected: Option<&str>,
) -> Result<InstructionsDoc, CoreError> {
    let doc = read_doc(folder, name)?;
    if !doc.editable {
        return Err(refuse(
            "notEditable",
            doc.note
                .unwrap_or_else(|| "the file cannot be edited".to_owned()),
        ));
    }
    match expected {
        None if doc.exists => Err(refuse(
            "alreadyExists",
            "the file now exists; reload and review it",
        )),
        Some(_) if !doc.exists => Err(refuse(
            "changedOnDisk",
            "the file was removed; reload and review it",
        )),
        Some(expected) if doc.text != expected => Err(refuse(
            "changedOnDisk",
            "the file changed; reload and review it",
        )),
        _ => Ok(doc),
    }
}

pub(crate) fn check_text(text: &str) -> Result<(), CoreError> {
    if text.len() > LIMIT || text.contains('\0') {
        return Err(invalid(
            "instruction text must be UTF-8, at most 256 KiB, without NUL",
        ));
    }
    Ok(())
}

/// Confirms the complete original text against a file opened through this same directory handle.
pub(crate) fn check_expected(dir: &Dir, leaf: &str, expected: &str) -> Result<(), CoreError> {
    let bytes = read_file(dir, leaf)
        .map_err(|_| refuse("changedOnDisk", "the file changed; reload and review it"))?;
    if bytes == expected.as_bytes() {
        Ok(())
    } else {
        Err(refuse(
            "changedOnDisk",
            "the file changed; reload and review it",
        ))
    }
}

pub(crate) fn write_existing(
    folder: &Path,
    name: &str,
    text: &str,
    expected: &str,
) -> Result<(), CoreError> {
    let (dir, leaf) = file_dir(folder, name, false)?;
    let tmp = format!(
        ".pi1-{}-{}.tmp",
        std::process::id(),
        TMP_NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut file = dir
        .open_with(&tmp, OpenOptions::new().write(true).create_new(true))
        .map_err(|error| io_error(&error))?;
    let result = (|| {
        file.write_all(text.as_bytes())
            .map_err(|error| io_error(&error))?;
        file.sync_all().map_err(|error| io_error(&error))?;
        check_expected(&dir, &leaf, expected)?;
        let permissions = dir
            .metadata(&leaf)
            .map_err(|error| io_error(&error))?
            .permissions();
        dir.set_permissions(&tmp, permissions)
            .map_err(|error| io_error(&error))?;
        dir.rename(&tmp, &dir, &leaf)
            .map_err(|error| io_error(&error))
    })();
    if result.is_err() {
        let _ = dir.remove_file(&tmp);
    }
    result
}
