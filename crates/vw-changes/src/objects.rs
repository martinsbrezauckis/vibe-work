//! Git objects and our index files, through commands that never read the working tree (so git cannot run working-tree filters or drivers).
//! review): `hash-object --no-filters`, `cat-file --batch`, `ls-files --stage`, `read-tree`, `update-index
//! --index-info`, `write-tree`, `diff-tree`. None of them runs a clean/smudge filter, a diff driver, an
//! external diff or a hook.
//!
//! Depends on: `git`, `repo`, `error`. Called by: `worktree`, `baseline`, `diff`, `review`.
//! Invariants: every call names one of our index files; blob content goes in and out as bytes.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::repo::Repo;

/// One index entry: mode (`100644`, `100755`, `120000`, `160000`) and object id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) mode: String,
    pub(crate) id: String,
}

/// Index entries by repository-relative path, as git's bytes (never decoded: a name that is not UTF-8 must
/// not fall out of the review).
pub(crate) type Entries = BTreeMap<Vec<u8>, Entry>;

/// The stage-0 entries of `index`.
pub(crate) fn read_index(repo: &Repo, index: &Path) -> Result<Entries, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["ls-files", "--stage", "-z"])
        .run()?;
    let mut entries = Entries::new();
    // Records: "<mode> <id> <stage>\t<path>\0" (no quoting with -z).
    for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| ChangesError::Parse("ls-files record without a tab".into()))?;
        let meta = String::from_utf8_lossy(&record[..tab]).into_owned();
        let path = record[tab + 1..].to_vec();
        let mut parts = meta.split(' ');
        let (Some(mode), Some(id), Some(stage)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(ChangesError::Parse(format!("ls-files record {meta:?}")));
        };
        if stage == "0" {
            entries.insert(
                path,
                Entry {
                    mode: mode.to_owned(),
                    id: id.to_owned(),
                },
            );
        }
    }
    Ok(entries)
}

fn index_info(entries: impl IntoIterator<Item = (Vec<u8>, Entry)>) -> Vec<u8> {
    let mut info = Vec::new();
    for (path, entry) in entries {
        info.extend_from_slice(format!("{} {}\t", entry.mode, entry.id).as_bytes());
        info.extend_from_slice(&path);
        info.push(0);
    }
    info
}

/// Replaces `index` with exactly `entries`.
pub(crate) fn write_index(
    repo: &Repo,
    index: &Path,
    entries: &Entries,
) -> Result<(), ChangesError> {
    match std::fs::remove_file(index) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(ChangesError::io("remove", index, e)),
    }
    Call::new(&repo.root, Index::Ours(index))
        .args(["read-tree", "--empty"])
        .run()?;
    let info = index_info(entries.iter().map(|(p, e)| (p.clone(), e.clone())));
    if !info.is_empty() {
        Call::new(&repo.root, Index::Ours(index))
            .args(["update-index", "-z", "--add", "--index-info"])
            .stdin(&info)
            .run()?;
    }
    Ok(())
}

/// Removes `removals` from `index` and adds or replaces `upserts`.
pub(crate) fn update_index(
    repo: &Repo,
    index: &Path,
    removals: &[Vec<u8>],
    upserts: &[(Vec<u8>, Entry)],
) -> Result<(), ChangesError> {
    if !removals.is_empty() {
        let mut list = Vec::new();
        for path in removals {
            list.extend_from_slice(path);
            list.push(0);
        }
        Call::new(&repo.root, Index::Ours(index))
            .args(["update-index", "-z", "--force-remove", "--stdin"])
            .stdin(&list)
            .run()?;
    }
    if !upserts.is_empty() {
        let info = index_info(upserts.iter().cloned());
        Call::new(&repo.root, Index::Ours(index))
            .args(["update-index", "-z", "--add", "--index-info"])
            .stdin(&info)
            .run()?;
    }
    Ok(())
}

/// `git write-tree` of `index`.
pub(crate) fn write_tree(repo: &Repo, index: &Path) -> Result<String, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["write-tree"])
        .run()?;
    let id = String::from_utf8_lossy(&out).trim().to_owned();
    if id.is_empty() {
        return Err(ChangesError::Parse("write-tree printed nothing".into()));
    }
    Ok(id)
}

/// Stores the files at `paths` (absolute, regular files) as blobs, byte for byte (`--no-filters`), and
/// returns their ids in order. Paths containing a newline are hashed one by one from bytes.
pub(crate) fn hash_files(
    repo: &Repo,
    index: &Path,
    paths: &[std::path::PathBuf],
) -> Result<Vec<String>, ChangesError> {
    let mut ids = vec![String::new(); paths.len()];
    let mut batch = Vec::new();
    let mut batch_at = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        // Absolute paths as raw bytes: they never start with `"` (which `--stdin-paths` would unquote), and
        // a name with a line break (which would split the list) is read and stored from bytes instead.
        let raw = crate::paths::os_bytes(path.as_os_str());
        if raw.contains(&b'\n') || raw.contains(&b'\r') || !path.is_absolute() {
            let bytes = std::fs::read(path).map_err(|e| ChangesError::io("read", path, e))?;
            ids[i] = hash_bytes(repo, index, &bytes)?;
        } else {
            batch.extend_from_slice(&raw);
            batch.push(b'\n');
            batch_at.push(i);
        }
    }
    if !batch_at.is_empty() {
        let out = Call::new(&repo.root, Index::Ours(index))
            .args(["hash-object", "-w", "--no-filters", "--stdin-paths"])
            .stdin(&batch)
            .run()?;
        let text = String::from_utf8_lossy(&out);
        let found: Vec<&str> = text.lines().collect();
        if found.len() != batch_at.len() {
            return Err(ChangesError::Parse(
                "hash-object printed the wrong number of ids".into(),
            ));
        }
        for (slot, id) in batch_at.into_iter().zip(found) {
            id.trim().clone_into(&mut ids[slot]);
        }
    }
    Ok(ids)
}

/// Stores `bytes` as a blob (`--no-filters`) and returns its id.
pub(crate) fn hash_bytes(repo: &Repo, index: &Path, bytes: &[u8]) -> Result<String, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["hash-object", "-w", "--no-filters", "--stdin"])
        .stdin(bytes)
        .run()?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// The id git gives a blob with `content` in a repository using `format` (`sha1` or `sha256`).
pub(crate) fn blob_id(format: &str, content: &[u8]) -> String {
    use sha1::Digest as _;
    let header = format!("blob {}\0", content.len());
    let hex = |bytes: &[u8]| {
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{b:02x}"));
            out
        })
    };
    if format == "sha256" {
        let mut hasher = sha2::Sha256::new();
        hasher.update(header.as_bytes());
        hasher.update(content);
        hex(&hasher.finalize())
    } else {
        let mut hasher = sha1::Sha1::new();
        hasher.update(header.as_bytes());
        hasher.update(content);
        hex(&hasher.finalize())
    }
}

/// The contents of the blobs `ids` (`cat-file --batch` runs no filter), each **verified against its id**
/// (integrity check: the agent can rewrite `.git/objects`; git does not check a loose object's
/// hash when it reads it). A mismatch, a corrupt object or any failure other than "missing" is
/// `BaselineLost` (fail closed: the core shows the read-only "starting point lost" view); a missing object
/// stays a git error so a refresh can first repair the baseline after a `gc --prune=now`.
pub(crate) fn read_blobs(
    repo: &Repo,
    index: &Path,
    ids: &[&str],
) -> Result<HashMap<String, Vec<u8>>, ChangesError> {
    let mut found = HashMap::new();
    if ids.is_empty() {
        return Ok(found);
    }
    let mut request = Vec::new();
    for id in ids {
        request.extend_from_slice(id.as_bytes());
        request.push(b'\n');
    }
    // Callers batch by size (`diff`); a revert reads one blob of any size the repository holds.
    let lost = |why: String| ChangesError::BaselineLost(why);
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["cat-file", "--batch"])
        .stdin(&request)
        .max_output(usize::MAX)
        .run()
        .map_err(|e| {
            lost(format!(
                "an object in the repository could not be read ({e})"
            ))
        })?;
    let items = match parse_batch(&out) {
        Ok(items) => items,
        Err(error) if error.is_missing_object() => return Err(error),
        Err(error) => {
            return Err(lost(format!(
                "an object in the repository could not be read ({error})"
            )));
        }
    };
    for (id, bytes) in items {
        if blob_id(&repo.object_format, &bytes) != id {
            return Err(lost(format!(
                "object {id} in the repository does not match its id (it was changed in .git/objects)"
            )));
        }
        found.insert(id, bytes);
    }
    Ok(found)
}

/// Parses `git cat-file --batch` output into `(id, content)` pairs.
pub(crate) fn parse_batch(out: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ChangesError> {
    let mut items = Vec::new();
    let mut at = 0;
    while at < out.len() {
        let end = out[at..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|p| at + p)
            .ok_or_else(|| ChangesError::Parse("cat-file header without newline".into()))?;
        let header = String::from_utf8_lossy(&out[at..end]).into_owned();
        let parts: Vec<&str> = header.split(' ').collect();
        if parts.len() == 2 && parts[1] == "missing" {
            return Err(ChangesError::Git {
                command: "cat-file".into(),
                message: format!("missing blob {}", parts[0]),
            });
        }
        if parts.len() != 3 {
            return Err(ChangesError::Parse(format!("cat-file header {header:?}")));
        }
        let size: usize = parts[2]
            .parse()
            .map_err(|_| ChangesError::Parse(format!("cat-file size {header:?}")))?;
        let body = end + 1;
        let stop = body + size;
        if stop > out.len() {
            return Err(ChangesError::Parse("cat-file output cut short".into()));
        }
        items.push((parts[0].to_owned(), out[body..stop].to_vec()));
        at = stop + 1;
    }
    Ok(items)
}
