//! Per-file streamed replacement and atomic persistence. Uses sha2 and tempfile; called by `redact()`.
//! A content fingerprint is checked once during copying; failures leave the original untouched.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    ops::Range,
};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    FileIssue, FileSkipReason, RedactionReport, ScanReport, scan::CHUNK_BYTES, types::Snapshot,
    validate,
};

pub(crate) fn redact(report: &ScanReport) -> RedactionReport {
    let mut result = RedactionReport::default();
    for snapshot in &report.files {
        match redact_file(snapshot, report.format_guard) {
            Ok(()) => result.redacted_files.push(snapshot.path.clone()),
            Err(reason) => result.skipped_files.push(FileIssue {
                file: snapshot.path.clone(),
                reason,
            }),
        }
    }
    result
}

fn redact_file(
    snapshot: &Snapshot,
    guard: Option<fn(&std::path::Path, &std::path::Path) -> bool>,
) -> Result<(), FileSkipReason> {
    let mut source = File::open(&snapshot.path).map_err(|_| FileSkipReason::Io)?;
    let metadata = source.metadata().map_err(|_| FileSkipReason::Io)?;
    let parent = snapshot.path.parent().ok_or(FileSkipReason::Io)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| FileSkipReason::Io)?;
    let mut ranges = snapshot.ranges.clone();
    ranges.sort_by_key(|range| range.start);
    let ranges = merge_ranges(ranges);
    let mut buffer = Zeroizing::new(vec![0; CHUNK_BYTES]);
    let mut hash = Sha256::new();
    let mut offset = 0;
    for range in ranges {
        transfer(
            &mut source,
            temporary.as_file_mut(),
            Some(range.start - offset),
            &mut hash,
            &mut buffer,
        )?;
        transfer(
            &mut source,
            &mut io::sink(),
            Some(range.end - range.start),
            &mut hash,
            &mut buffer,
        )?;
        temporary
            .write_all(b"[redacted]")
            .map_err(|_| FileSkipReason::Io)?;
        offset = range.end;
    }
    transfer(
        &mut source,
        temporary.as_file_mut(),
        None,
        &mut hash,
        &mut buffer,
    )?;
    let actual: [u8; 32] = hash.finalize().into();
    if actual != snapshot.hash {
        return Err(FileSkipReason::ChangedSinceScan);
    }
    let jsonl = snapshot.path.extension().is_some_and(|ext| ext == "jsonl");
    temporary
        .seek(SeekFrom::Start(0))
        .map_err(|_| FileSkipReason::Io)?;
    if let Some(guard) = guard {
        if !guard(&snapshot.path, temporary.path()) {
            return Err(FileSkipReason::InvalidJson);
        }
    } else {
        validate::validate(temporary.as_file_mut(), jsonl)?;
    }
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| FileSkipReason::Io)?;
    // Closing the input also permits replacement on Windows.
    drop(source);
    temporary
        .persist(&snapshot.path)
        .map_err(|_| FileSkipReason::Io)?;
    Ok(())
}

pub(crate) fn merge_ranges(ranges: Vec<Range<u64>>) -> Vec<Range<u64>> {
    let mut merged: Vec<Range<u64>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn transfer(
    source: &mut File,
    target: &mut impl Write,
    mut remaining: Option<u64>,
    hash: &mut Sha256,
    buffer: &mut [u8],
) -> Result<(), FileSkipReason> {
    loop {
        let length = remaining.map_or(buffer.len(), |bytes| {
            usize::try_from(bytes.min(buffer.len() as u64)).unwrap_or(buffer.len())
        });
        if length == 0 {
            return Ok(());
        }
        let read = source
            .read(&mut buffer[..length])
            .map_err(|_| FileSkipReason::Io)?;
        if read == 0 {
            return if remaining.is_some() {
                Err(FileSkipReason::ChangedSinceScan)
            } else {
                Ok(())
            };
        }
        hash.update(&buffer[..read]);
        target
            .write_all(&buffer[..read])
            .map_err(|_| FileSkipReason::Io)?;
        remaining = remaining.map(|bytes| bytes - read as u64);
    }
}
