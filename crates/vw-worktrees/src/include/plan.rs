//! The plan: which ignored files of the main checkout a `.worktreeinclude` selects, before anything is copied
//! (the new-chat panel shows it; the copy re-plans right before writing).
//!
//! Depends on: `source` (git's ignored listing, safe reads), `pattern`, `names`. Called by: [`super::apply`]'s
//! callers (vw-core-api), `worktrees options`.
//! Invariants:
//! - A file is selected only if git lists it as ignored (so a tracked file never is) and a pattern selects it.
//! - Wholly ignored directories come from git as one entry and are walked only when a pattern can reach into
//!   them ([`Patterns::may_reach`]); the walk never follows a link and stops at [`MAX_VISITED`] entries.
//! - Over a cap the plan is empty and says so: a half-filled copy would look complete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::names::validate_relative;
use super::pattern::Patterns;
use super::source::{
    Ignored, IncludeFileState, ignore_case, is_link_like, list_ignored, read_include_file,
};
use super::{
    INCLUDE_FILE, IncludeReport, MAX_BYTES, MAX_DEPTH, MAX_FILES, MAX_VISITED, SkipReason,
};

/// What the walk has seen so far.
struct Walk<'a> {
    root: &'a Path,
    patterns: &'a Patterns,
    /// Other working trees inside the checkout (never copied from).
    other_trees: &'a [PathBuf],
    report: IncludeReport,
    visited: usize,
    /// Why the plan must be emptied (a cap).
    over: Option<String>,
}

/// Plans the copy for the checkout at `root`. `other_trees` are the repository's other working trees (the new
/// copy included when it already exists): files below them are never sources.
///
/// Never fails: every problem is in the report (`stopped`, `skipped`).
#[must_use]
pub fn plan(root: &Path, other_trees: &[PathBuf]) -> IncludeReport {
    let mut report = IncludeReport::default();
    let text = match read_include_file(root) {
        IncludeFileState::Missing => return report,
        IncludeFileState::Unusable(why) => {
            report.found = true;
            report.stopped = Some(why);
            return report;
        }
        IncludeFileState::Patterns(text) => text,
    };
    report.found = true;
    let patterns = Patterns::parse(&text, ignore_case(root));
    if patterns.is_empty() {
        return report;
    }
    let (ignored, bad_names) = match list_ignored(root) {
        Ok(listing) => listing,
        Err(why) => {
            report.stopped = Some(why);
            return report;
        }
    };
    for name in bad_names {
        report.skip(name, SkipReason::NotText);
    }
    let mut walk = Walk {
        root,
        patterns: &patterns,
        other_trees,
        report,
        visited: 0,
        over: None,
    };
    // git prints a directory whose files are all ignored AND those files one by one (the directory is not
    // itself ignored); only a directory printed alone is wholly ignored (matched by an ignore rule), the
    // case the `**/` rule is about.
    let mut listed_below: HashSet<&str> = HashSet::new();
    for entry in &ignored {
        if let Ignored::File(path) = entry {
            for (i, _) in path.match_indices('/') {
                listed_below.insert(&path[..i]);
            }
        }
    }
    for entry in &ignored {
        if walk.over.is_some() {
            break;
        }
        match entry {
            Ignored::File(path) => walk.file(path, None),
            Ignored::Dir(path) if !listed_below.contains(path.as_str()) => walk.dir_entry(path),
            Ignored::Dir(_) => {}
        }
    }
    let mut report = walk.report;
    if let Some(why) = walk.over {
        report.files.clear();
        report.bytes = 0;
        report.stopped = Some(why);
    }
    report.files.sort_by(|a, b| a.path.cmp(&b.path));
    report
}

impl Walk<'_> {
    /// A file git listed (`inside` = how many leading names are a wholly ignored directory).
    fn file(&mut self, rel: &str, inside: Option<usize>) {
        let comps: Vec<&str> = rel.split('/').collect();
        if !self.patterns.selects(&comps, inside) {
            return;
        }
        self.take(rel, &comps);
    }

    /// The selected file `rel`: name, location and type checks, then the caps.
    fn take(&mut self, rel: &str, comps: &[&str]) {
        if validate_relative(rel).is_err() {
            self.report.skip(rel, SkipReason::UnsafeName);
            return;
        }
        let abs = comps.iter().fold(self.root.to_path_buf(), |p, c| p.join(c));
        if self.other_trees.iter().any(|t| abs.starts_with(t)) {
            self.report.skip(rel, SkipReason::InsideWorktree);
            return;
        }
        let size = match super::safefs::open_source(self.root, comps) {
            Ok((_, meta)) => meta.len(),
            Err(reason) => {
                self.report.skip(rel, reason);
                return;
            }
        };
        self.report.add(rel.to_owned(), size);
        if self.report.files.len() > MAX_FILES {
            self.over = Some(format!(
                "More than {MAX_FILES} files match {INCLUDE_FILE}; nothing was copied. Name fewer files (a folder such as node_modules is an install, not something to copy)."
            ));
        } else if u64::from(self.report.bytes) > MAX_BYTES || size > MAX_BYTES {
            self.over = Some(format!(
                "The files that match {INCLUDE_FILE} are larger than {} MB together; nothing was copied. Name fewer files.",
                MAX_BYTES >> 20
            ));
        }
    }

    /// A wholly ignored directory git printed as one entry.
    fn dir_entry(&mut self, rel: &str) {
        let comps: Vec<&str> = rel.split('/').collect();
        if validate_relative(rel).is_err() {
            // Not walked: its name could point anywhere on another system. Listed only if a pattern named it.
            if self.patterns.selects(&comps, None) {
                self.report.skip(rel, SkipReason::UnsafeName);
            }
            return;
        }
        let selected = self.patterns.selects(&comps, None);
        if !selected && !self.patterns.may_reach(&comps) {
            return;
        }
        self.descend(&comps, comps.len(), 0);
    }

    /// Walks `comps` (a directory), selecting files with `inside` leading names being the ignored directory.
    fn descend(&mut self, comps: &[&str], inside: usize, depth: usize) {
        if depth > MAX_DEPTH || self.over.is_some() {
            return;
        }
        let dir = comps.iter().fold(self.root.to_path_buf(), |p, c| p.join(c));
        let Ok(read) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut names: Vec<(String, std::fs::Metadata)> = Vec::new();
        for entry in read.flatten() {
            self.visited += 1;
            if self.visited > MAX_VISITED {
                self.over = Some(format!(
                    "{INCLUDE_FILE} reaches into a folder with more than {MAX_VISITED} entries; nothing was copied. Name the files instead of the folder."
                ));
                return;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                self.report
                    .skip(entry.file_name().to_string_lossy(), SkipReason::NotText);
                continue;
            };
            if let Ok(meta) = std::fs::symlink_metadata(entry.path()) {
                names.push((name, meta));
            }
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, meta) in names {
            // A `.git` (a nested repository's directory, a worktree's link file) is never a source and not
            // worth listing.
            if name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let mut child: Vec<&str> = comps.to_vec();
            child.push(&name);
            let rel = child.join("/");
            if is_link_like(&meta) {
                if self.patterns.selects(&child, Some(inside)) {
                    self.report.skip(rel, SkipReason::Link);
                }
            } else if meta.is_dir() {
                self.descend(&child, inside, depth + 1);
            } else {
                self.file(&rel, Some(inside));
            }
            if self.over.is_some() {
                return;
            }
        }
    }
}
