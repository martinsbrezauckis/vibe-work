//! The PR card without gh: GitHub's public REST API for public repositories,
//! unauthenticated (60 requests an hour per address). One refresh = up to 5 requests (find the PR by head, the
//! PR, the head commit's check runs and status, the reviews). The budget is read from `x-ratelimit-remaining`;
//! below 10 left, no request is made until `x-ratelimit-reset`, and the card says so. A repository the API
//! cannot see (private, or no such branch) gets "Open on GitHub". No token is ever sent.
//!
//! Depends on: vw-mistral (`https_client`: the core's HTTPS client, no redirects), vw-git (`pr::from_rest`).
//! Called by: `git_publish` (`prStatus` with `refresh`).
//! Invariants: GET only, to `CoreConfig::github_api_base` (tests point it at a local fake); a 10 s limit per
//! request.

use std::sync::PoisonError;
use std::time::Duration;

use serde_json::Value;
use vw_git::forge::Forge;

use crate::core::Shared;
use crate::git_cmd::CardCache;
use crate::git_view::PrSource;

/// Below this many requests left in the hour, the card stops asking until the reset.
const LOW_BUDGET: u64 = 10;

struct Answer {
    body: Option<Value>,
    remaining: Option<u64>,
    reset_s: Option<u64>,
    status: u16,
}

async fn get(client: &reqwest::Client, url: &str) -> Option<Answer> {
    let response = client
        .get(url)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "vibe-work")
        .header("x-github-api-version", "2022-11-28")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let (remaining, reset_s) = (header("x-ratelimit-remaining"), header("x-ratelimit-reset"));
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(b.as_ref()).ok());
    Some(Answer {
        body,
        remaining,
        reset_s,
        status,
    })
}

/// Refreshes `cached` from the public API (or explains why not). `now` is ms since the epoch.
pub(crate) async fn fetch(
    shared: &Shared,
    forge: &Forge,
    head: &str,
    cached: CardCache,
    now: u64,
) -> CardCache {
    let resume = *shared
        .git
        .api_resume_ms
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if now < resume {
        return CardCache {
            note: Some(format!(
                "GitHub's public API limit is nearly used up; the card refreshes again in about {} minutes. Install and \
                 sign in to gh for live updates.",
                (resume - now) / 60_000 + 1
            )),
            ..cached
        };
    }
    let Ok(client) = vw_mistral::https_client() else {
        return CardCache {
            note: Some("HTTPS is not available.".to_owned()),
            ..cached
        };
    };
    let base = shared.config.github_api_base.trim_end_matches('/');
    let repo = format!("{base}/repos/{}", forge.path);
    let owner = forge.path.split('/').next().unwrap_or("");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("head", &format!("{owner}:{head}"))
        .append_pair("state", "all")
        .append_pair("per_page", "1")
        .finish();
    let mut budget: Option<(u64, Option<u64>)> = None;
    let mut track = |a: &Answer| {
        if let Some(left) = a.remaining {
            budget = Some((left, a.reset_s));
        }
    };
    let failed = |why: &str| CardCache {
        source: Some(PrSource::None),
        fetched_at_ms: now,
        note: Some(why.to_owned()),
        ..cached.clone()
    };
    let Some(list) = get(&client, &format!("{repo}/pulls?{query}")).await else {
        return failed("GitHub could not be reached.");
    };
    track(&list);
    let result = if list.status == 404 {
        failed(
            "GitHub's public API cannot see this repository (private?). Sign in to gh, or open it on GitHub.",
        )
    } else if list.status != 200 {
        failed(&format!("GitHub answered {}.", list.status))
    } else {
        match list
            .body
            .as_ref()
            .and_then(|b| b.get(0))
            .and_then(|p| p.get("number"))
            .and_then(Value::as_u64)
        {
            None => CardCache {
                card: None,
                source: Some(PrSource::PublicApi),
                fetched_at_ms: now,
                note: None,
            },
            Some(number) => {
                let (card, seen) = read_pull(&client, &repo, number).await;
                for answer in seen {
                    track(&answer);
                }
                CardCache {
                    card,
                    source: Some(PrSource::PublicApi),
                    fetched_at_ms: now,
                    note: None,
                }
            }
        }
    };
    if let Some((left, reset)) = budget
        && left < LOW_BUDGET
    {
        let until = reset.map_or(now + 3_600_000, |s| s.saturating_mul(1000));
        *shared
            .git
            .api_resume_ms
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = until;
    }
    result
}

/// The card of pull request `number`: the PR, its head commit's check runs and status, its reviews.
async fn read_pull(
    client: &reqwest::Client,
    repo: &str,
    number: u64,
) -> (Option<vw_git::pr::PrCard>, Vec<Answer>) {
    let mut seen = Vec::new();
    let pull = get(client, &format!("{repo}/pulls/{number}")).await;
    let pull_body = pull.and_then(|a| {
        let body = a.body.clone();
        seen.push(Answer { body: None, ..a });
        body
    });
    let sha = pull_body
        .as_ref()
        .and_then(|p| p.pointer("/head/sha"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut parts = Vec::new();
    for url in [
        format!("{repo}/commits/{sha}/check-runs?per_page=100"),
        format!("{repo}/commits/{sha}/status"),
        format!("{repo}/pulls/{number}/reviews?per_page=100"),
    ] {
        let answer = get(client, &url).await;
        parts.push(answer.and_then(|a| {
            let body = a.body.clone();
            seen.push(Answer { body: None, ..a });
            body
        }));
    }
    let card = pull_body.as_ref().and_then(|p| {
        vw_git::pr::from_rest(p, parts[0].as_ref(), parts[1].as_ref(), parts[2].as_ref())
    });
    (card, seen)
}

/// The failure and warning annotations of one check run from the public API (the failing lines of
/// "Send failing checks to Vibe" without gh; job logs themselves need a signed-in account). One request, within the
/// same hourly budget as the card.
///
/// # Errors
/// Why there are none to quote (budget, no HTTPS, an answer other than 200).
pub(crate) async fn annotations(
    shared: &Shared,
    forge: &Forge,
    check_run: u64,
) -> Result<Option<String>, String> {
    let now = vw_changes::now_ms();
    let resume = *shared
        .git
        .api_resume_ms
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if now < resume {
        return Err(
            "GitHub's public API limit is nearly used up; sign in to gh to include the log"
                .to_owned(),
        );
    }
    let client = vw_mistral::https_client().map_err(|_| "HTTPS is not available".to_owned())?;
    let base = shared.config.github_api_base.trim_end_matches('/');
    let url = format!(
        "{base}/repos/{}/check-runs/{check_run}/annotations?per_page=50",
        forge.path
    );
    let answer = get(&client, &url)
        .await
        .ok_or_else(|| "GitHub's API could not be reached".to_owned())?;
    if answer.remaining.is_some_and(|r| r < LOW_BUDGET)
        && let Some(reset) = answer.reset_s
    {
        *shared
            .git
            .api_resume_ms
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = reset.saturating_mul(1000);
    }
    if answer.status != 200 {
        return Err(format!(
            "GitHub's API answered {} (a private repository needs gh, signed in)",
            answer.status
        ));
    }
    Ok(answer
        .body
        .as_ref()
        .and_then(vw_git::checklog::from_annotations))
}
