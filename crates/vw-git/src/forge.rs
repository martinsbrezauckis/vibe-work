//! Which forge a remote lives on, and the forge pages it opens in the system browser: the prefilled "new pull
//! request" page and a branch's pull requests.
//!
//! Supported pages: GitHub `compare/<base>...<head>?quick_pull=1&title=&body=` (no draft
//! parameter exists: Draft is chosen on the page; an over-long address is answered `414`, so the body is left
//! out above [`MAX_URL`] and GitHub fills its template); GitLab `-/merge_requests/new` with source and target
//! branch (title from the commit); Codeberg/Forgejo `compare/<base>...<head>` (its page fills the title from
//! a single commit and the PR template; it reads no title/body parameters, `routers/web/repo/compare.go`).
//! Other hosts get no page: the UI says to open the forge.
//!
//! Depends on: url (form encoding), serde. Called by: `pr`, vw-core-api.
//! Invariants: pure functions; an address is shown only without its user-info part (no token in the UI).

use serde::{Deserialize, Serialize};

/// Longest address handed to the browser (GitHub answers `414 URI Too Long` above its limit).
pub const MAX_URL: usize = 7000;

/// The forge kind of a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ForgeKind {
    /// github.com.
    GitHub,
    /// gitlab.com.
    GitLab,
    /// codeberg.org (Forgejo).
    Forgejo,
    /// Anything else (a local path, a self-hosted server we cannot tell apart).
    Unknown,
}

/// A remote's repository on a forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Forge {
    /// The kind.
    pub kind: ForgeKind,
    /// Host name, lower case (empty for a local path).
    pub host: String,
    /// `owner/repo` (GitLab: `group/sub/repo`), without `.git`.
    pub path: String,
}

impl Forge {
    /// `HOST/OWNER/REPO` for `gh --repo`.
    #[must_use]
    pub fn gh_repo(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }

    /// The repository's web address.
    #[must_use]
    pub fn web(&self) -> String {
        format!("https://{}/{}", self.host, self.path)
    }
}

/// Reads `url` (a remote address as git prints it): `https://[user@]host[:port]/owner/repo[.git]`,
/// `ssh://[user@]host[:port]/owner/repo[.git]`, `git@host:owner/repo[.git]`. `None` for a local path or
/// something else.
#[must_use]
pub fn parse_remote(url: &str) -> Option<Forge> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
        .or_else(|| url.strip_prefix("git://"))
    {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        (host.to_owned(), path.to_owned())
    } else if !url.contains("://") && url.contains(':') && !looks_like_windows_path(url) {
        // scp-like `[user@]host:owner/repo.git`
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit('@').next()?;
        (host.to_owned(), path.trim_start_matches('/').to_owned())
    } else {
        return None;
    };
    let host = host.to_ascii_lowercase();
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).to_owned();
    let segments = path.split('/').filter(|s| !s.is_empty()).count();
    // Owner and repository names as forges allow them: letters, digits, `.`, `_`, `-`, never a leading `-`
    // or `.` (so no part of it can be read as an option or a relative path).
    let plain = path.split('/').all(|s| {
        !s.is_empty()
            && !s.starts_with(['-', '.'])
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });
    let host_ok = !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if !host_ok || !plain || segments < 2 || path.contains("..") {
        return None;
    }
    let kind = match host.as_str() {
        "github.com" | "www.github.com" => ForgeKind::GitHub,
        "gitlab.com" => ForgeKind::GitLab,
        "codeberg.org" => ForgeKind::Forgejo,
        _ => ForgeKind::Unknown,
    };
    let host = if host == "www.github.com" {
        "github.com".to_owned()
    } else {
        host
    };
    Some(Forge { kind, host, path })
}

fn looks_like_windows_path(url: &str) -> bool {
    let bytes = url.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `url` without a user-info part (`https://user:token@host/x` → `https://host/x`), for display.
#[must_use]
pub fn display_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, tail) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, ""),
        };
        if let Some((_, host)) = authority.rsplit_once('@') {
            // ssh's `git@` is a user name, not a secret, but one rule for all is simpler to audit.
            return format!("{scheme}://{host}{tail}");
        }
    }
    url.to_owned()
}

/// A branch name as a path segment of a forge address (`/` kept: forges accept it in branch names).
fn branch_segment(name: &str) -> String {
    name.split('/')
        .map(|part| {
            url::form_urlencoded::byte_serialize(part.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The forge page that creates a pull/merge request from `head` into `base`, prefilled where the forge
/// allows it. `None` for `ForgeKind::Unknown`.
#[must_use]
pub fn new_request_url(
    forge: &Forge,
    base: &str,
    head: &str,
    title: &str,
    body: &str,
) -> Option<String> {
    if !crate::names::valid_ref_name(base) || !crate::names::valid_ref_name(head) {
        return None;
    }
    match forge.kind {
        ForgeKind::GitHub => {
            let start = format!(
                "{}/compare/{}...{}?quick_pull=1",
                forge.web(),
                branch_segment(base),
                branch_segment(head)
            );
            let title_part = query("title", title);
            let with_body = format!("{start}&{title_part}&{}", query("body", body));
            if body.is_empty() || with_body.len() > MAX_URL {
                let without = format!("{start}&{title_part}");
                if without.len() > MAX_URL {
                    return Some(start);
                }
                return Some(without);
            }
            Some(with_body)
        }
        ForgeKind::GitLab => Some(format!(
            "{}/-/merge_requests/new?{}&{}",
            forge.web(),
            query("merge_request[source_branch]", head),
            query("merge_request[target_branch]", base)
        )),
        ForgeKind::Forgejo => Some(format!(
            "{}/compare/{}...{}",
            forge.web(),
            branch_segment(base),
            branch_segment(head)
        )),
        ForgeKind::Unknown => None,
    }
}

/// True when [`new_request_url`] for this forge carries the body (GitHub and only when it fits).
#[must_use]
pub fn url_carries_body(url: &str) -> bool {
    url.contains("&body=")
}

/// The forge page listing pull requests of `head` (the card's "Open on GitHub" without a known number).
#[must_use]
pub fn branch_requests_url(forge: &Forge, head: &str) -> Option<String> {
    if !crate::names::valid_ref_name(head) {
        return None;
    }
    match forge.kind {
        ForgeKind::GitHub => Some(format!(
            "{}/pulls?{}",
            forge.web(),
            query("q", &format!("is:pr head:{head}"))
        )),
        ForgeKind::GitLab => Some(format!(
            "{}/-/merge_requests?{}",
            forge.web(),
            query("source_branch", head)
        )),
        ForgeKind::Forgejo => Some(format!("{}/pulls", forge.web())),
        ForgeKind::Unknown => None,
    }
}

fn query(key: &str, value: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair(key, value)
        .finish()
}
