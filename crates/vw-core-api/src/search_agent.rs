//! Agent text/glob search over the Files engine; called by the session MCP adapter.
//! Only approved regular files are returned, with bounded output and work.
use crate::search_files::{Folder, MAX_BYTES, MAX_FILES};
use crate::search_walk;
use globset::GlobBuilder;
use regex::RegexBuilder;
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub query: Option<String>,
    #[serde(default)]
    pub regex: bool,
    pub glob: Option<String>,
    pub limit: Option<usize>,
}

pub(crate) fn run(
    folders: &[Folder],
    request: &Request,
    allowed: &dyn Fn(&Path) -> bool,
    check_opened: &dyn Fn(&std::fs::File, &Path) -> bool,
    budget: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + budget;
    let (pattern, glob) = prepare(request)?;
    let limit = request.limit.unwrap_or(50).clamp(1, 200);
    let mut hits = Vec::new();
    let mut total = 0;
    let mut scanned = 0;
    let mut bytes = 0;
    let mut truncated = false;
    'folders: for (folder_index, folder) in folders.iter().enumerate() {
        let (names, cut) = search_walk::candidates(&folder.path, deadline, allowed);
        truncated |= cut;
        for rel in names {
            if rel.len() > 2048 {
                truncated = true;
                continue;
            }
            if Instant::now() >= deadline
                || scanned >= MAX_FILES
                || bytes >= MAX_BYTES
                || total >= 1000
            {
                truncated = true;
                break 'folders;
            }
            if glob.as_ref().is_some_and(|g| !g.is_match(&rel)) || !allowed(&folder.path.join(&rel))
            {
                continue;
            }
            // Check the opened object's path too, before either a name-only hit or a byte read.
            let Some(file) = open_allowed(&folder.path, &rel, allowed, check_opened)
                .filter(|_| Instant::now() < deadline)
            else {
                continue;
            };
            if pattern.is_none() {
                total += 1;
                if hits.len() < limit {
                    hits.push((folder_index, rel));
                }
                continue;
            }
            let Some(content) = search_walk::read_opened(file) else {
                continue;
            };
            scanned += 1;
            bytes += content.len() as u64;
            let Ok(text) = std::str::from_utf8(&content) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if Instant::now() >= deadline || total >= 1000 {
                    truncated = true;
                    break 'folders;
                }
                if let Some(matched) = pattern.as_ref().and_then(|p| p.find(line)) {
                    total += 1;
                    if hits.len() < limit {
                        let start = line[..matched.start()]
                            .char_indices()
                            .rev()
                            .nth(50)
                            .map_or(0, |(at, _)| at);
                        let snippet: String = line[start..]
                            .chars()
                            .take(500)
                            .map(|c| if c.is_control() { ' ' } else { c })
                            .collect();
                        let cut = start > 0 || line[start..].chars().count() > 500;
                        hits.push((
                            folder_index,
                            format!(
                                "{rel}:{}: {snippet}{}",
                                index + 1,
                                if cut { "…" } else { "" }
                            ),
                        ));
                    }
                }
            }
        }
    }
    let kept = hits.len();
    let mut output = Vec::new();
    let mut previous = None;
    for (index, line) in hits {
        if folders.len() > 1 && previous != Some(index) {
            output.push(format!("Folder: {}", folders[index].path.display()));
            previous = Some(index);
        }
        output.push(line);
    }
    if total > kept {
        output.push(format!("{} more hits not shown", total - kept));
    }
    if truncated || Instant::now() >= deadline {
        output.push("Search limit reached.".into());
    }
    if output.is_empty() {
        output.push("No hits.".into());
    }
    Ok(output.join("\n"))
}

// The same handle supplies the bytes after its final path passes the caller's policy.
fn open_allowed(
    folder: &Path,
    relative: &str,
    allowed: &dyn Fn(&Path) -> bool,
    check_opened: &dyn Fn(&std::fs::File, &Path) -> bool,
) -> Option<std::fs::File> {
    let file = search_walk::open(folder, relative).ok()?.into_std();
    let opened = vw_platform::final_path::final_path(&file).ok()?;
    (allowed(&opened) && check_opened(&file, &folder.join(relative))).then_some(file)
}

#[cfg(test)]
#[path = "search_agent_tests.rs"]
mod tests;

fn prepare(
    request: &Request,
) -> Result<(Option<regex::Regex>, Option<globset::GlobMatcher>), String> {
    let query = request.query.as_deref().filter(|q| !q.is_empty());
    if query.is_none() && request.glob.is_none() {
        return Err("query or glob is required".into());
    }
    if query.is_some_and(|s| s.len() > 4096)
        || request.glob.as_ref().is_some_and(|s| s.len() > 4096)
    {
        return Err("query or glob is too long".into());
    }
    let pattern = query
        .map(|q| {
            let source = if request.regex {
                q.to_owned()
            } else {
                regex::escape(q)
            };
            RegexBuilder::new(&source)
                .size_limit(1 << 20)
                .build()
                .map_err(|_| "Invalid regular expression".to_owned())
        })
        .transpose()?;
    let glob = request
        .glob
        .as_deref()
        .map(|g| {
            GlobBuilder::new(g)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|_| "Invalid file glob".to_owned())
        })
        .transpose()?;
    Ok((pattern, glob))
}
