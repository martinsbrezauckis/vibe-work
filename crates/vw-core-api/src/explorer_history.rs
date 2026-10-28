//! Stable, fifty-commit history pages and lazy changed-path lists. Depends on the hardened git runner; called by Files.
use crate::error::{CoreError, ErrorCode};
use crate::explorer_read::{failed, git, relative};
use crate::explorer_view::{ExplorerResult, ExplorerVersion};
use std::path::Path;

fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn history(
    root: &Path,
    cursor: Option<&str>,
    query: &str,
    path: Option<&str>,
) -> Result<ExplorerResult, CoreError> {
    if query.len() > 512 || path.is_some_and(|p| !relative(p)) {
        return Err(failed());
    }
    let (head, skip) = if let Some(cursor) = cursor {
        let (head, skip) = cursor.split_once(':').ok_or_else(failed)?;
        let skip = skip
            .parse::<u32>()
            .ok()
            .filter(|n| *n <= 1_000_000)
            .ok_or_else(failed)?;
        if !oid(head) {
            return Err(failed());
        }
        (head.to_owned(), skip)
    } else {
        let head = git(root, &["rev-parse", "--verify", "HEAD"], 256).unwrap_or_default();
        let head = String::from_utf8_lossy(&head).trim().to_owned();
        if !oid(&head) {
            return Ok(ExplorerResult::History {
                versions: Vec::new(),
                cursor: None,
            });
        }
        (head, 0)
    };
    if !query.is_empty() {
        return search_versions(root, &head, query);
    }
    let skip_arg = format!("--skip={skip}");
    let mut args = vec![
        "log",
        "--format=%H%x00%s%x00%an%x00%ae%x00%at",
        "-z",
        "-51",
        &skip_arg,
        &head,
        "--",
    ];
    // A literal path (no pattern magic): the commits that changed this one file.
    let literal = path.map(|p| format!(":(literal){p}"));
    if let Some(literal) = literal.as_deref() {
        args.push(literal);
    }
    let out = git(root, &args, 512 << 10)?;
    let mut versions = parse_versions(root, &out);
    let more = versions.len() > 50;
    versions.truncate(50);
    mark_publication(root, &mut versions);
    Ok(ExplorerResult::History {
        versions,
        cursor: more.then(|| format!("{head}:{}", skip + 50)),
    })
}
fn parse_versions(root: &Path, out: &[u8]) -> Vec<ExplorerVersion> {
    let own = git(root, &["config", "user.email"], 4096).unwrap_or_default();
    let own = String::from_utf8_lossy(&own).trim().to_owned();
    let fields: Vec<_> = out.split(|b| *b == 0).collect();
    fields
        .as_chunks::<5>()
        .0
        .iter()
        .filter_map(|chunk| {
            let text = |n: usize| String::from_utf8_lossy(chunk[n]).trim().to_owned();
            let id = text(0);
            if !oid(&id) {
                return None;
            }
            Some(ExplorerVersion {
                id,
                subject: text(1),
                who: if !own.is_empty() && own.eq_ignore_ascii_case(&text(3)) {
                    "you".into()
                } else {
                    text(2)
                },
                time: text(4).parse().unwrap_or(0.0),
                published: Some(false),
            })
        })
        .collect()
}

fn search_versions(root: &Path, head: &str, query: &str) -> Result<ExplorerResult, CoreError> {
    let mut matches = std::collections::HashMap::new();
    // Separate native filters implement message OR author, including commits far back in the history.
    for flag in ["--grep", "--author"] {
        let filter = format!("{flag}={query}");
        let bytes = git(
            root,
            &[
                "log",
                "--fixed-strings",
                "--regexp-ignore-case",
                "--format=%H%x00%s%x00%an%x00%ae%x00%at",
                "-z",
                "-51",
                &filter,
                head,
                "--",
            ],
            512 << 10,
        )?;
        for version in parse_versions(root, &bytes) {
            matches.insert(version.id.clone(), version);
        }
    }
    let mut versions: Vec<_> = matches.into_values().collect();
    versions.sort_by(|a, b| b.time.total_cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
    versions.truncate(50);
    mark_publication(root, &mut versions);
    Ok(ExplorerResult::History {
        versions,
        cursor: None,
    })
}

fn mark_publication(root: &Path, versions: &mut [ExplorerVersion]) {
    // One bounded upstream walk marks the visible page. No fetch or forge request occurs.
    if let Ok(upstream) = git(root, &["rev-parse", "--verify", "@{upstream}"], 256) {
        let upstream = String::from_utf8_lossy(&upstream).trim().to_owned();
        if oid(&upstream)
            && let Ok(ids) = git(
                root,
                &["rev-list", "--max-count=100000", &upstream],
                8 << 20,
            )
        {
            let ids = String::from_utf8_lossy(&ids);
            let known: std::collections::HashSet<_> = ids.lines().collect();
            for version in versions {
                version.published = if known.contains(version.id.as_str()) {
                    Some(true)
                } else if known.len() < 100_000 {
                    Some(false)
                } else {
                    None
                };
            }
        }
    }
}

pub(crate) fn version(root: &Path, id: &str) -> Result<ExplorerResult, CoreError> {
    if !oid(id) {
        return Err(failed());
    }
    let bytes = git(
        root,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            id,
            "--",
        ],
        4 << 20,
    )?;
    let mut files: Vec<_> = bytes
        .split(|b| *b == 0)
        .filter_map(|b| String::from_utf8(b.to_vec()).ok())
        .filter(|p| !p.is_empty() && relative(p))
        .take(5001)
        .collect();
    let truncated = files.len() > 5000;
    files.truncate(5000);
    Ok(ExplorerResult::Version { files, truncated })
}

/// A workspace review uses its own scratch baseline against HEAD, leaving session baselines intact.
pub(crate) fn review(
    root: &Path,
    scratch: &Path,
) -> Result<(vw_changes::Baseline, vw_changes::Diff), CoreError> {
    let repo = vw_changes::discover(root)
        .map_err(crate::changes_view::core_error)?
        .ok_or_else(failed)?;
    let id = format!(
        "files-{}",
        vw_changes::fnv_hex(&[root.to_string_lossy().as_bytes()])
    );
    let dir =
        vw_changes::SessionDir::open(scratch, &id).map_err(crate::changes_view::core_error)?;
    let baseline = vw_changes::against_head(repo, &dir).map_err(crate::changes_view::core_error)?;
    let diff = vw_changes::refresh(&baseline).map_err(crate::changes_view::core_error)?;
    Ok((baseline, diff))
}
pub(crate) fn revert(
    root: &Path,
    scratch: &Path,
    head: &str,
    fingerprint: &str,
    delete_paths: &[String],
) -> Result<ExplorerResult, CoreError> {
    let current = git(root, &["rev-parse", "--verify", "HEAD"], 256).unwrap_or_default();
    if String::from_utf8_lossy(&current).trim() != head {
        return Err(CoreError::new(
            ErrorCode::Stale,
            "The last commit changed. Review the changes again before reverting.",
        ));
    }
    let (baseline, _) = review(root, scratch)?;
    vw_changes::revert(
        &baseline,
        &vw_changes::Target::All {
            fingerprint: fingerprint.to_owned(),
        },
        delete_paths,
    )
    .map_err(crate::changes_view::core_error)?;
    Ok(ExplorerResult::Done {})
}

/// One file's text at commit `id`, read from the object store (no checkout, no filters, no hooks).
pub(crate) fn read_at(root: &Path, id: &str, path: &str) -> Result<ExplorerResult, CoreError> {
    if !oid(id) || !relative(path) {
        return Err(failed());
    }
    // `<commit>:./<path>` is relative to the folder Files shows, which may sit inside the repository.
    let (text, note) = crate::files_saved::blob_text(
        root,
        &format!("{id}:./{path}"),
        "This file is not in that commit.",
    );
    Ok(ExplorerResult::Text { text, note })
}
