//! The Git panel's pull request list: every pull request whose head is one of the chat's branches (the branch
//! of its folder and, when the chat has a copy record, the copy's branch), newest first, each with its status line.
//! From the person's own gh when it is signed in (open ones also get their required checks named), else GitHub's
//! public REST API (state only; the note says checks need gh).
//!
//! Depends on: vw-git (`gh`, `pr`, `pr_status`, `state`), `git_api` (public API list), `git_cmd` (place, cache).
//! Called by: `git_publish` (`prList`).
//! Invariants: the forge is asked only on `refresh` (the window refreshes only while the list is on screen) and
//! never more often than the card's floor (30 s gh, 120 s API, or `CoreConfig::pr_card_floor`). No background
//! watcher: a later one would call the same function on a schedule the person sets.

use vw_git::forge::ForgeKind;
use vw_git::pr::{PrCard, PrState};
use vw_git::state;
use vw_platform::sync::LockRecover;

use crate::core::Core;
use crate::error::CoreError;
use crate::git_cmd::{Place, git_error};
use crate::git_view::{PrListView, PrSource};
use crate::worktrees::blocking;

/// The shortest time between two gh refreshes of one list (the card's floor).
const GH_FLOOR_MS: u64 = 30_000;
/// The shortest time between two public-API refreshes of one list.
const API_FLOOR_MS: u64 = 120_000;
/// Open pull requests whose required checks are read per refresh (one gh call each).
const REQUIRED_READS: usize = 5;

/// One chat's list as last read.
#[derive(Debug, Clone, Default)]
pub(crate) struct ListCache {
    pub(crate) view: Option<PrListView>,
    pub(crate) fetched_clock: Option<crate::clock::ClockReading>,
}

/// Newest first, each pull request once.
fn merged(mut cards: Vec<PrCard>) -> Vec<PrCard> {
    cards.sort_by_key(|c| std::cmp::Reverse(c.number));
    cards.dedup_by_key(|c| c.number);
    cards
}

impl Core {
    /// `prList`.
    pub(crate) async fn pr_list_request(
        &self,
        request: crate::git_view::GitRequest,
    ) -> Result<crate::git_view::GitResult, CoreError> {
        let crate::git_view::GitRequest::PrList {
            session_id,
            repo,
            refresh,
        } = request
        else {
            unreachable!("only prList is routed here");
        };
        let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
        Ok(crate::git_view::GitResult::PrList {
            list: self.pr_list(&session_id, &place, refresh).await?,
        })
    }

    pub(crate) async fn pr_list(
        &self,
        session_id: &str,
        place: &Place,
        refresh: bool,
    ) -> Result<PrListView, CoreError> {
        let here = place.clone();
        let state = blocking(move || state::read_state(&here.root, &here.git_dir, None))
            .await?
            .map_err(git_error)?;
        let mut branches: Vec<String> = state.branch().into_iter().map(str::to_owned).collect();
        if let Some(copy) = self
            .shared
            .worktrees
            .records()
            .ok()
            .and_then(|r| r.get(session_id))
            .map(|r| r.branch)
            && !branches.contains(&copy)
        {
            branches.push(copy);
        }
        let key = format!("{session_id}:{}", place.root.display());
        let cached = self
            .shared
            .git
            .lists
            .lock_or_recover()
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let empty = |source: PrSource, note: Option<String>| PrListView {
            cards: Vec::new(),
            branches: branches.clone(),
            source,
            fetched_at_ms: 0,
            note,
        };
        let Some(forge) = state
            .default_remote()
            .and_then(|r| r.forge.clone())
            .filter(|f| f.kind == ForgeKind::GitHub)
        else {
            return Ok(empty(
                PrSource::None,
                Some("The list reads GitHub pull requests.".to_owned()),
            ));
        };
        let now = crate::clock::ClockReading::now();
        let built_in = if cached.view.as_ref().map(|v| v.source) == Some(PrSource::PublicApi) {
            API_FLOOR_MS
        } else {
            GH_FLOOR_MS
        };
        let floor = self.shared.config.pr_card_floor.map_or(built_in, |d| {
            u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
        });
        let within = cached.fetched_clock.is_some_and(|at| {
            now.instant.saturating_duration_since(at.instant)
                < std::time::Duration::from_millis(floor)
        });
        if !refresh || branches.is_empty() || within {
            return Ok(cached.view.unwrap_or_else(|| empty(PrSource::None, None)));
        }
        let signed_in = self.gh_signed_in(&forge.host).await;
        let view = if signed_in {
            self.list_with_gh(&forge, &branches, now)
                .await?
                .unwrap_or_else(|e| PrListView {
                    note: Some(format!("gh could not list the pull requests: {e}")),
                    fetched_at_ms: now.wall_ms,
                    ..cached
                        .view
                        .clone()
                        .unwrap_or_else(|| empty(PrSource::Gh, None))
                })
        } else {
            let (cards, note) = crate::git_api::list_pulls(&self.shared, &forge, &branches).await;
            PrListView {
                cards: merged(cards),
                branches: branches.clone(),
                source: PrSource::PublicApi,
                fetched_at_ms: now.wall_ms,
                note,
            }
        };
        self.shared.git.lists.lock_or_recover().insert(
            key,
            ListCache {
                view: Some(view.clone()),
                fetched_clock: Some(now),
            },
        );
        Ok(view)
    }
    /// The person's gh is installed and signed in to `host`.
    async fn gh_signed_in(&self, host: &str) -> bool {
        let Some(gh) = self.gh() else {
            return false;
        };
        let host = host.to_owned();
        blocking(move || gh.signed_in(&host))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }

    /// The list from gh, with the required checks of the first open pull requests named.
    async fn list_with_gh(
        &self,
        forge: &vw_git::forge::Forge,
        branches: &[String],
        now: crate::clock::ClockReading,
    ) -> Result<Result<PrListView, vw_git::GitError>, CoreError> {
        let Some(gh) = self.gh() else {
            return Ok(Err(vw_git::GitError::Gh("gh is not installed".to_owned())));
        };
        let (on, heads) = (forge.clone(), branches.to_vec());
        let read = blocking(move || {
            let mut cards = Vec::new();
            for head in &heads {
                cards.extend(gh.list(&on, head)?);
            }
            let mut cards = merged(cards);
            for card in cards
                .iter_mut()
                .filter(|c| c.state == PrState::Open && !c.draft)
                .take(REQUIRED_READS)
            {
                if let Ok(names) = gh.required_checks(&on, card.number) {
                    vw_git::pr_status::mark_required(card, &names);
                }
            }
            Ok::<_, vw_git::GitError>(cards)
        })
        .await?;
        Ok(read.map(|cards| PrListView {
            cards,
            branches: branches.to_vec(),
            source: PrSource::Gh,
            fetched_at_ms: now.wall_ms,
            note: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::merged;

    fn card(number: u64) -> vw_git::pr::PrCard {
        vw_git::pr::from_gh(&serde_json::json!({"number": number, "state": "OPEN"})).unwrap()
    }

    #[test]
    fn pr_list_merges_copies_and_dedupes() {
        let list = merged(vec![card(3), card(9), card(3), card(5)]);
        let numbers: Vec<u64> = list.iter().map(|c| c.number).collect();
        assert_eq!(numbers, [9, 5, 3]);
    }
}
