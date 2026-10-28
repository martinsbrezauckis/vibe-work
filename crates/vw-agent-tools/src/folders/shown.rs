//! Paths in folder guard refusals as people write them: the reason leads a refusal and the path follows in brackets,
//! without Windows' `\\?\` prefix (the bell shows the start of the line). Called by `folders::Layout::check`.

use std::path::Path;

/// `path` without the `\\?\` prefix Windows adds to canonical paths (unchanged elsewhere).
pub(super) fn shown(path: &Path) -> std::path::Display<'_> {
    dunce::simplified(path).display()
}
