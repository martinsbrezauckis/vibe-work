//! Batched file-reference lookup and a short session-folder listing cache.
//! Depends on `files_scope` and `search_files`; called by files on a blocking worker. Cached names grant no access.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::files_scope::{Hit, Scope, resolve};

const TTL: Duration = Duration::from_secs(10);
const FOLDERS: usize = 4;

#[derive(Default)]
pub(crate) struct ListingCache {
    entries: VecDeque<(PathBuf, Instant, Arc<Vec<String>>)>,
}

impl ListingCache {
    fn get(
        &mut self,
        folder: &Path,
        now: Instant,
        list: &mut impl FnMut(&Path) -> Vec<String>,
    ) -> Arc<Vec<String>> {
        let key = crate::changes_facts::path_key(folder);
        self.entries
            .retain(|(_, at, _)| now.saturating_duration_since(*at) < TTL);
        if let Some(at) = self.entries.iter().position(|(p, _, _)| p == &key) {
            let entry = self.entries.remove(at);
            if let Some(entry) = entry {
                let names = Arc::clone(&entry.2);
                self.entries.push_back(entry);
                return names;
            }
        }
        let names = Arc::new(list(folder));
        if self.entries.len() == FOLDERS {
            self.entries.pop_front();
        }
        self.entries.push_back((key, now, Arc::clone(&names)));
        names
    }
}

fn suffix(path: &str, name: &str, windows: bool) -> bool {
    // The platform helper is also used by changes_facts::path_key and the scope's containment gate.
    let key = |s: &str| {
        if windows {
            vw_platform::windows_key(s).replace('\\', "/")
        } else {
            s.to_owned()
        }
    };
    let (path, name) = (key(path), key(name));
    path == name
        || path
            .strip_suffix(&name)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

pub(crate) fn lookup(
    scope: &Scope,
    paths: Vec<String>,
    cache: &mut ListingCache,
    now: Instant,
    mut list: impl FnMut(&Path) -> Vec<String>,
) -> Vec<(String, Vec<Hit>)> {
    let mut names = None;
    // A fallback may never use picked folders, including through a symlink into a picked folder. The listed names
    // are relative to the session folder, so they are never joined to the working folder.
    let own = Scope {
        picked: vec![],
        cwd: None,
        ..scope.clone()
    };
    paths
        .into_iter()
        .map(|text| {
            let hits = if let Some(hit) = resolve(scope, &text) {
                vec![hit]
            } else if let Some(name) = relative_name(&text) {
                let names = names.get_or_insert_with(|| cache.get(&scope.session, now, &mut list));
                names
                    .iter()
                    .filter(|rel| suffix(rel, &name, cfg!(windows)))
                    .filter_map(|rel| resolve(&own, rel))
                    .collect()
            } else {
                vec![]
            };
            (text, hits)
        })
        .collect()
}

/// No filesystem access, and no reinterpretation of traversal, absolute, home, drive or URL text.
fn relative_name(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty()
        || text.len() > crate::files_scope::MAX_PATH_TEXT
        || text.chars().any(char::is_control)
        || text.starts_with(['~', '/', '\\'])
        || text.contains(':')
        || text.split(['/', '\\']).any(|p| p == "..")
    {
        return None;
    }
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.to_owned()
    };
    let parts: Vec<_> = text
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual listing timing"]
    fn repository_listing_time() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let data = std::env::temp_dir().join(format!("listing-timing-{}", std::process::id()));
        vw_changes::prepare_root(&data).unwrap();
        let started = Instant::now();
        let names = crate::search_files::candidates(&folder);
        println!(
            "repository listing: {} names in {:.3} ms",
            names.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn suffix_comparison_is_segment_exact_and_uses_the_windows_key() {
        assert!(suffix(
            "Chrome Plugin/TEST/Commands.PY",
            r"test\commands.py",
            true
        ));
        assert!(!suffix("p/xtest/commands.py", "test/commands.py", true));
        assert!(!suffix("p/Commands.py", "commands.py", false));
        assert!(!suffix("p/xcommands.py", "commands.py", false));
    }

    #[test]
    fn cache_reuses_expires_and_bounds_folders() {
        let mut cache = ListingCache::default();
        let now = Instant::now();
        let mut calls = 0;
        let mut list = |_: &Path| {
            calls += 1;
            vec!["p/commands.py".into()]
        };
        let a = cache.get(Path::new("a"), now, &mut list);
        let b = cache.get(Path::new("a"), now + Duration::from_secs(9), &mut list);
        assert!(
            Arc::ptr_eq(&a, &b),
            "the second call must use the cached listing"
        );
        let c = cache.get(Path::new("a"), now + TTL, &mut list);
        assert!(!Arc::ptr_eq(&a, &c), "the listing must expire");
        for p in ["b", "c", "d", "e"] {
            cache.get(Path::new(p), now + TTL, &mut list);
        }
        cache.get(Path::new("a"), now + TTL, &mut list);
        assert_eq!(calls, 7);
        assert_eq!(cache.entries.len(), FOLDERS);
    }

    #[test]
    fn plain_misses_list_once_but_traversal_absolute_home_drive_and_urls_never_list() {
        let folder = std::env::temp_dir().join(format!("file-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let scope = Scope {
            session: folder.clone(),
            cwd: None,
            picked: vec![],
            home: None,
            hidden: true,
        };
        let mut calls = 0;
        let mut cache = ListingCache::default();
        for text in [
            "../x.py",
            "p/../x.py",
            "/outside.py",
            "~/x.py",
            "~x.py",
            r"C:\x.py",
            "C:x.py",
            r"\x.py",
            "file:///x.py",
        ] {
            lookup(
                &scope,
                vec![text.into()],
                &mut cache,
                Instant::now(),
                |_| {
                    calls += 1;
                    vec![]
                },
            );
        }
        assert_eq!(calls, 0, "ineligible text must never trigger a listing");
        lookup(
            &scope,
            vec!["missing.py".into(), "other.py".into()],
            &mut cache,
            Instant::now(),
            |_| {
                calls += 1;
                vec![]
            },
        );
        assert_eq!(calls, 1, "one listing for the entire batch");
        lookup(
            &scope,
            vec!["missing.py".into()],
            &mut cache,
            Instant::now(),
            |_| {
                calls += 1;
                vec![]
            },
        );
        assert_eq!(calls, 1, "the next batch uses the cache");
        std::fs::remove_dir_all(folder).unwrap();
    }
}
