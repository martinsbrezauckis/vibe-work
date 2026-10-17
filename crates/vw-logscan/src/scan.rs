//! Chunked byte matching and file fingerprints. Uses memchr, sha2 and walkdir; called by `scan()`.
//! Buffers retain only one chunk and the longest pattern's overlap, never a complete session.

use std::{fs, io::Read, path::Path};

use memchr::{memchr_iter, memmem};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zeroize::Zeroizing;

use crate::{
    Error, FileIssue, FileSkipReason, Location, ScanReport, Secret,
    patterns::{Pattern, patterns},
    types::Snapshot,
};

pub(crate) const CHUNK_BYTES: usize = 64 * 1024;

pub(crate) fn scan(folder: &Path, secrets: Vec<Secret>) -> Result<ScanReport, Error> {
    let metadata = fs::symlink_metadata(folder).map_err(|error| Error::Folder(error.kind()))?;
    if !metadata.is_dir() {
        return Err(Error::Folder(std::io::ErrorKind::NotADirectory));
    }
    let folder = folder
        .canonicalize()
        .map_err(|error| Error::Folder(error.kind()))?;
    let files = WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry)
                if entry.file_type().is_file()
                    && matches!(
                        entry.path().extension().and_then(|ext| ext.to_str()),
                        Some("json" | "jsonl")
                    ) =>
            {
                Some(Ok(entry.into_path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(Error::Folder(
                error
                    .io_error()
                    .map_or(std::io::ErrorKind::Other, std::io::Error::kind),
            ))),
        });
    scan_files(files, secrets)
}

pub(crate) fn scan_files(
    files: impl IntoIterator<Item = Result<std::path::PathBuf, Error>>,
    secrets: Vec<Secret>,
) -> Result<ScanReport, Error> {
    let patterns = patterns(&secrets)?;
    let mut report = ScanReport {
        secrets: secrets.iter().map(Secret::report).collect(),
        skipped_files: Vec::new(),
        files: Vec::new(),
        format_guard: None,
    };
    // Raw values are no longer needed; the encoded patterns wipe on return or error too.
    drop(secrets);
    let Some(longest) = patterns.iter().map(|pattern| pattern.bytes.len()).max() else {
        return Ok(report);
    };
    let mut buffer = Zeroizing::new(vec![0; CHUNK_BYTES + longest - 1]);
    for entry in files {
        let path = entry?;
        if !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("json" | "jsonl")
        ) {
            continue;
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => continue,
            Err(_) => {
                report.skipped_files.push(FileIssue {
                    file: path,
                    reason: FileSkipReason::Io,
                });
                continue;
            }
        }
        match scan_file(&path, &patterns, longest, &mut buffer) {
            Ok((hash, matches)) => {
                if matches.is_empty() {
                    continue;
                }
                let ranges = matches
                    .iter()
                    .map(|(_, location)| {
                        location.byte_offset..location.byte_offset + location.byte_length
                    })
                    .collect();
                report.files.push(Snapshot { path, hash, ranges });
                for (secret, location) in matches {
                    report.secrets[secret].locations.push(location);
                }
            }
            Err(_) => report.skipped_files.push(FileIssue {
                file: path,
                reason: FileSkipReason::Io,
            }),
        }
    }
    for secret in &mut report.secrets {
        secret
            .locations
            .sort_by(|a, b| (&a.file, a.byte_offset).cmp(&(&b.file, b.byte_offset)));
    }
    Ok(report)
}

type FileMatches = ([u8; 32], Vec<(usize, Location)>);

fn scan_file(
    path: &Path,
    patterns: &[Pattern],
    longest: usize,
    buffer: &mut [u8],
) -> std::io::Result<FileMatches> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut matches = Vec::new();
    let mut used = 0;
    let mut offset = 0;
    let mut line = 1;
    loop {
        let read = file.read(&mut buffer[used..])?;
        hash.update(&buffer[used..used + read]);
        used += read;
        let safe = if read == 0 {
            used
        } else {
            used.saturating_sub(longest - 1)
        };
        for pattern in patterns {
            let mut cursor = 0;
            while let Some(found) = memmem::find(&buffer[cursor..used], &pattern.bytes) {
                let start = cursor + found;
                if start >= safe {
                    break;
                }
                matches.push((
                    pattern.secret,
                    Location {
                        file: path.to_path_buf(),
                        line: line + memchr_iter(b'\n', &buffer[..start]).count() as u64,
                        byte_offset: offset + start as u64,
                        byte_length: pattern.bytes.len() as u64,
                        encoding: pattern.encoding,
                    },
                ));
                // Restart one byte later to include overlapping occurrences.
                cursor = start + 1;
            }
        }
        if read == 0 {
            // Padded and unpadded base64 can match at the same start; retain the full range.
            matches.sort_by(|(id_a, a), (id_b, b)| {
                (id_a, a.byte_offset, std::cmp::Reverse(a.byte_length)).cmp(&(
                    id_b,
                    b.byte_offset,
                    std::cmp::Reverse(b.byte_length),
                ))
            });
            matches.dedup_by(|(id_a, a), (id_b, b)| id_a == id_b && a.byte_offset == b.byte_offset);
            return Ok((hash.finalize().into(), matches));
        }
        line += memchr_iter(b'\n', &buffer[..safe]).count() as u64;
        offset += safe as u64;
        buffer.copy_within(safe..used, 0);
        used -= safe;
    }
}
