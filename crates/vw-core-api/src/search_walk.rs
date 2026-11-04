//! Shared Files/agent listing and bounded reads. Git supplies tracked/unignored names; a confined
//! fallback walk respects nested gitignores. Handles refuse links and Windows reparse points.
use crate::search_files::{MAX_FILE, MAX_FILES, SKIP_DIRS, plain_relative};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn linked(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub(crate) fn root(folder: &Path) -> io::Result<Dir> {
    let mut base = PathBuf::new();
    let mut relative = PathBuf::new();
    for part in folder.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => base.push(part.as_os_str()),
            Component::Normal(name) => relative.push(name),
            _ => return Err(io::ErrorKind::PermissionDenied.into()),
        }
    }
    if !base.is_absolute() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let anchor = Dir::open_ambient_dir(base, cap_std::ambient_authority())?;
    directory(&anchor, &relative)
}
/// Opens each child through its verified parent, refusing links and Windows reparse points.
pub(crate) fn directory(root: &Dir, relative: &Path) -> io::Result<Dir> {
    let mut dir = root.try_clone()?;
    for part in relative.components() {
        if part == Component::CurDir {
            continue;
        }
        if let Component::Normal(name) = part {
            if linked(&dir.symlink_metadata(name)?) {
                return Err(io::Error::other("path contains a link or reparse point"));
            }
            dir = dir.open_dir_nofollow(name)?;
            if linked(&dir.dir_metadata()?) {
                return Err(io::Error::other("path contains a link or reparse point"));
            }
        } else {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(dir)
}
/// Opens a regular file through no-follow parent handles, even if a listed parent was replaced.
pub(crate) fn open(folder: &Path, rel: &str) -> io::Result<File> {
    if !plain_relative(rel) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let root = root(folder)?;
    let path = Path::new(rel);
    let parent = directory(&root, path.parent().unwrap_or(Path::new("")))?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let meta = parent.symlink_metadata(name)?;
    if linked(&meta) {
        return Err(io::Error::other("path contains a link or reparse point"));
    }
    if !meta.is_file() {
        return Err(io::Error::other("not a file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options)?;
    let meta = file.metadata()?;
    if linked(&meta) || !meta.is_file() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}
/// Size is checked on the opened handle and again after the capped read (a file may grow).
pub(crate) fn read(folder: &Path, rel: &str) -> Option<Vec<u8>> {
    read_opened(open(folder, rel).ok()?.into_std())
}
/// Reads the same handle whose identity the caller checked.
pub(crate) fn read_opened(file: std::fs::File) -> Option<Vec<u8>> {
    if file.metadata().ok()?.len() > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_FILE && !bytes[..bytes.len().min(8192)].contains(&0))
        .then_some(bytes)
}
/// One shared listing, capped during enumeration as well as during content reads.
pub(crate) fn candidates(
    folder: &Path,
    deadline: Instant,
    allowed: &dyn Fn(&Path) -> bool,
) -> (Vec<String>, bool) {
    if Instant::now() >= deadline || !allowed(folder) {
        return (Vec::new(), Instant::now() >= deadline);
    }
    let listed = vw_changes::git::Call::new(folder, vw_changes::git::Index::User)
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(3)),
        )
        .max_output(4 << 20)
        .output();
    if let Ok(out) = listed
        && out.ok()
    {
        if out.stdout.is_empty() {
            // An enclosing repository may ignore a separately picked folder. Its own ignores still apply.
            let enclosing_ignored = std::fs::symlink_metadata(folder.join(".git")).is_err()
                && Instant::now() < deadline
                && vw_changes::git::Call::new(folder, vw_changes::git::Index::User)
                    .args(["check-ignore", "-q", "."])
                    .timeout(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_secs(1)),
                    )
                    .max_output(1024)
                    .output()
                    .is_ok_and(|out| out.ok());
            return if enclosing_ignored {
                walk(folder, deadline, allowed)
            } else {
                (Vec::new(), false)
            };
        }
        let mut names = Vec::new();
        for bytes in out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            if names.len() >= MAX_FILES as usize || Instant::now() >= deadline {
                return (names, true);
            }
            let rel = String::from_utf8_lossy(bytes).into_owned();
            if visible(&rel) && allowed(&folder.join(&rel)) {
                names.push(rel);
            }
        }
        names.sort();
        names.dedup();
        return (names, false);
    }
    walk(folder, deadline, allowed)
}
fn visible(rel: &str) -> bool {
    plain_relative(rel)
        && !rel
            .split('/')
            .any(|p| p.starts_with('.') || SKIP_DIRS.contains(&p))
}
fn walk(folder: &Path, deadline: Instant, allowed: &dyn Fn(&Path) -> bool) -> (Vec<String>, bool) {
    let Ok(root) = root(folder) else {
        return (Vec::new(), false);
    };
    let mut names = Vec::new();
    let mut stack = vec![(PathBuf::new(), Vec::<Arc<Gitignore>>::new())];
    let mut entries = 0;
    while let Some((relative, mut ignores)) = stack.pop() {
        if Instant::now() >= deadline {
            return (names, true);
        }
        let Ok(dir) = directory(&root, &relative) else {
            continue;
        };
        let ignore_name = relative.join(".gitignore");
        let has_ignore = match dir.symlink_metadata(".gitignore") {
            Ok(meta) => meta.is_file() && !linked(&meta),
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(_) => return (names, true),
        };
        if has_ignore {
            // Dropping unread rules could expose ignored files. An exhausted policy budget stops the scan.
            if !allowed(&folder.join(&ignore_name)) {
                return (names, true);
            }
            let Some(bytes) = read(folder, &ignore_name.to_string_lossy()) else {
                return (names, true);
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return (names, true);
            };
            let mut builder = GitignoreBuilder::new(folder.join(&relative));
            for (index, line) in text.lines().enumerate() {
                if index >= 4096 || Instant::now() >= deadline {
                    return (names, true);
                }
                let _ = builder.add_line(None, line);
            }
            let Ok(ignore) = builder.build() else {
                return (names, true);
            };
            ignores.push(Arc::new(ignore));
        }
        let Ok(children) = dir.entries() else {
            continue;
        };
        for child in children.flatten() {
            entries += 1;
            if entries > MAX_FILES * 2 || Instant::now() >= deadline {
                names.sort();
                return (names, true);
            }
            let rel = relative.join(child.file_name());
            let text = rel.to_string_lossy().replace('\\', "/");
            if !visible(&text) || !allowed(&folder.join(&rel)) {
                continue;
            }
            let Ok(meta) = dir.symlink_metadata(child.file_name()) else {
                continue;
            };
            if linked(&meta) {
                continue;
            }
            let path = folder.join(&rel);
            if ignores
                .iter()
                .rev()
                .map(|i| i.matched(&path, meta.is_dir()))
                .find(|m| !m.is_none())
                .is_some_and(|m| m.is_ignore())
            {
                continue;
            }
            if meta.is_dir() {
                if rel.components().count() < 32 {
                    stack.push((rel, ignores.clone()));
                }
            } else if meta.is_file() {
                names.push(text);
            }
        }
    }
    names.sort();
    (names, false)
}
