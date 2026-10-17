//! Hashes the bytes a copy removal discards, including listed ignored folders and nested repositories.
//! Depends on cap-std, BLAKE3 and handle-relative Unix opens; called by inspect. Reads stay inside the
//! copy, symlink targets are hashed without following them, and exceeding a budget refuses removal.

use std::io::Read;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use cap_std::fs::Dir;

use super::{LostFile, WorktreeError};

const MAX_BYTES: u64 = 256 << 20;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_TIME: Duration = Duration::from_secs(30);

struct Scan {
    hash: blake3::Hasher,
    bytes: u64,
    entries: usize,
    began: Instant,
}

fn limited() -> WorktreeError {
    WorktreeError::NotOffered(
        "The copy is too large to inspect safely for removal. Keep it or remove it yourself."
            .into(),
    )
}

fn io(path: &Path, error: std::io::Error) -> WorktreeError {
    WorktreeError::io("inspect copy contents", path, error)
}

fn open_dir(dir: &Dir, name: &Path) -> std::io::Result<Dir> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, openat};
        let fd = openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Dir::from_std_file(std::fs::File::from(fd)))
    }
    #[cfg(not(unix))]
    {
        dir.open_dir(name)
    }
}

fn open_file(dir: &Dir, name: &Path) -> std::io::Result<cap_std::fs::File> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, openat};
        let fd = openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(cap_std::fs::File::from_std(std::fs::File::from(fd)))
    }
    #[cfg(not(unix))]
    {
        dir.open(name)
    }
}

impl Scan {
    fn part(&mut self, bytes: &[u8]) {
        self.hash.update(&(bytes.len() as u64).to_le_bytes());
        self.hash.update(bytes);
    }

    fn check(&self) -> Result<(), WorktreeError> {
        if self.bytes > MAX_BYTES || self.entries > MAX_ENTRIES || self.began.elapsed() > MAX_TIME {
            Err(limited())
        } else {
            Ok(())
        }
    }

    fn visit(&mut self, dir: &Dir, name: &Path, depth: usize) -> Result<(), WorktreeError> {
        self.entries += 1;
        self.check()?;
        if depth > MAX_DEPTH {
            return Err(limited());
        }
        self.part(name.as_os_str().as_encoded_bytes());
        let meta = match dir.symlink_metadata(name) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.part(b"missing");
                return Ok(());
            }
            Err(error) => return Err(io(name, error)),
        };
        if meta.is_symlink() {
            self.part(b"link");
            self.part(
                dir.read_link_contents(name)
                    .map_err(|error| io(name, error))?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
        } else if meta.is_dir() {
            self.part(b"directory");
            let child = open_dir(dir, name).map_err(|error| io(name, error))?;
            let mut names = Vec::new();
            for entry in child.entries().map_err(|error| io(name, error))? {
                names.push(entry.map_err(|error| io(name, error))?.file_name());
                if names.len() + self.entries > MAX_ENTRIES {
                    return Err(limited());
                }
                self.check()?;
            }
            names.sort();
            for entry in names {
                self.visit(&child, Path::new(&entry), depth + 1)?;
            }
            self.part(b"end directory");
        } else if meta.is_file() {
            self.part(b"file");
            let mut file = open_file(dir, name).map_err(|error| io(name, error))?;
            let opened = file.metadata().map_err(|error| io(name, error))?;
            if !opened.is_file() {
                return Err(limited());
            }
            if opened.len() > MAX_BYTES.saturating_sub(self.bytes) {
                return Err(limited());
            }
            let mut buffer = vec![0; 64 << 10].into_boxed_slice();
            let mut size = 0_u64;
            loop {
                let read = file.read(&mut buffer).map_err(|error| io(name, error))?;
                if read == 0 {
                    break;
                }
                self.bytes += read as u64;
                size += read as u64;
                self.check()?;
                self.hash.update(&buffer[..read]);
            }
            self.part(&size.to_le_bytes());
            let after = file.metadata().map_err(|error| io(name, error))?;
            if opened.len() != size || opened.modified().ok() != after.modified().ok() {
                return Err(WorktreeError::Stale(
                    "A copy file changed during inspection; check it again.".into(),
                ));
            }
        } else {
            return Err(WorktreeError::NotOffered("The copy contains a special file that cannot be inspected safely. Keep it or remove it yourself.".into()));
        }
        Ok(())
    }
}

pub(super) fn fingerprint(root: &Path, files: &[LostFile]) -> Result<String, WorktreeError> {
    let root = Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| io(root, error))?;
    let mut scan = Scan {
        hash: blake3::Hasher::new(),
        bytes: 0,
        entries: 0,
        began: Instant::now(),
    };
    'files: for file in files {
        let path = Path::new(file.path.trim_end_matches('/'));
        let mut parts = path.components().collect::<Vec<_>>();
        if parts.is_empty()
            || parts.len() > MAX_DEPTH
            || parts
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(limited());
        }
        let leaf = parts.pop().ok_or_else(limited)?;
        let mut parent = root.try_clone().map_err(|error| io(path, error))?;
        for part in parts {
            parent = match open_dir(&parent, Path::new(part.as_os_str())) {
                Ok(parent) => parent,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    scan.part(file.path.as_bytes());
                    scan.part(b"missing");
                    continue 'files;
                }
                Err(error) => return Err(io(path, error)),
            };
        }
        scan.part(file.path.as_bytes());
        scan.visit(
            &parent,
            Path::new(leaf.as_os_str()),
            path.components().count(),
        )?;
    }
    Ok(scan.hash.finalize().to_hex().to_string())
}
