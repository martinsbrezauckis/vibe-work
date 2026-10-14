//! The PR card of the `git` command: the card from gh, else GitHub's public REST
//! API (`git_api.rs`), cached per session with a floor between forge reads, and the prepared composer messages
//! ("Send failing checks to Vibe", "Send review comments to Vibe", "Ask Vibe to update the copy").
//!
//! "Send failing checks" carries each failing check's lines (`checklog`): gh's failed-step log
//! when gh is signed in, else the check run's annotations from the public API, else a sentence saying why not.
//!
//! Depends on: vw-git (`pr`, `checklog`, `forge`, `gh`, `merge`), vw-secrets (scrub), `git_cmd` (place, cache), `git_api`. Called by: `git_publish`,
//! `git_pr` (after a PR is made).
//! Invariant: the forge is asked only on `refresh` and never more often than the floor (30 s gh, 120 s API, or
//! `CoreConfig::pr_card_floor`).
//! Prepared messages label repository/forge content as untrusted data; vw-git's builders quote metadata and every
//! body line, and never interpolate a branch into a shell command. The UI inserts a draft; the user sends it.

use vw_git::checklog::{self, CheckLog};
use vw_git::forge::{self, ForgeKind};
use vw_git::merge;
use vw_git::pr::{self, PrCard};
use vw_git::state;

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::{CardCache, Place, git_error};
use crate::git_view::{PrSource, PrStatusView, PreparedKind};
use crate::worktrees::blocking;

/// The shortest time between two gh refreshes of one card.
const GH_FLOOR_MS: u64 = 30_000;
/// The shortest time between two public-API refreshes of one card (60 requests an hour, up to 5 per refresh).
const API_FLOOR_MS: u64 = 120_000;

impl Core {
    pub(crate) async fn pr_status(
        &self,
        session_id: &str,
        place: &Place,
        refresh: bool,
    ) -> Result<PrStatusView, CoreError> {
        let here = place.clone();
        let state = blocking(move || state::read_state(&here.root, &here.git_dir, None))
            .await?
            .map_err(git_error)?;
        let head = state.branch().unwrap_or("").to_owned();
        let forge = state.default_remote().and_then(|r| r.forge.clone());
        let open_url = forge
            .as_ref()
            .and_then(|f| forge::branch_requests_url(f, &head));
        let mut cached = self.shared.git.card(session_id, &place.root);
        if cached.card.as_ref().is_some_and(|card| card.head != head) {
            cached = CardCache::default();
        }
        let view = |c: &CardCache| PrStatusView {
            card: c.card.clone(),
            source: c.source.unwrap_or(PrSource::None),
            fetched_at_ms: c.fetched_at_ms,
            note: c.note.clone(),
            open_url: c
                .card
                .as_ref()
                .map(|card| card.url.clone())
                .or_else(|| open_url.clone()),
        };
        let Some(forge) = forge.filter(|f| f.kind == ForgeKind::GitHub) else {
            let mut card = cached;
            card.note = Some(
                "The PR card reads GitHub pull requests; open your forge for this remote."
                    .to_owned(),
            );
            return Ok(view(&card));
        };
        let now = vw_changes::now_ms();
        let built_in = if cached.source == Some(PrSource::PublicApi) {
            API_FLOOR_MS
        } else {
            GH_FLOOR_MS
        };
        let floor = self.shared.config.pr_card_floor.map_or(built_in, |d| {
            u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
        });
        if !refresh || head.is_empty() || now.saturating_sub(cached.fetched_at_ms) < floor {
            return Ok(view(&cached));
        }
        let fresh = match self.gh() {
            Some(gh)
                if {
                    let (g, h) = (gh.clone(), forge.host.clone());
                    blocking(move || g.signed_in(&h))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false)
                } =>
            {
                let (on, branch) = (forge.clone(), head.clone());
                match blocking(move || gh.view(&on, &branch)).await? {
                    Ok(card) => CardCache {
                        card,
                        source: Some(PrSource::Gh),
                        fetched_at_ms: now,
                        note: None,
                    },
                    Err(e) => CardCache {
                        note: Some(format!("gh could not read the pull request: {e}")),
                        fetched_at_ms: now,
                        ..cached.clone()
                    },
                }
            }
            _ => crate::git_api::fetch(&self.shared, &forge, &head, cached.clone(), now).await,
        };
        self.shared
            .git
            .cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                format!("{session_id}:{}", place.root.display()),
                fresh.clone(),
            );
        Ok(view(&fresh))
    }

    pub(crate) async fn prepared(
        &self,
        session_id: &str,
        place: &Place,
        what: PreparedKind,
    ) -> Result<String, CoreError> {
        match what {
            PreparedKind::UpdateFromBase => Ok(merge::update_from_base_message(
                &self.bring_plan(place).await?.plan,
            )),
            PreparedKind::Checks | PreparedKind::Review => {
                let card: PrCard = self
                    .shared
                    .git
                    .card(session_id, &place.root)
                    .card
                    .ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::NotFound,
                            "no pull request is known for this branch yet",
                        )
                    })?;
                if what == PreparedKind::Checks {
                    return Ok(self.checks_with_logs(&card, place).await);
                }
                let p = place.clone();
                let (comments, lines_unread) = match (
                    self.gh(),
                    blocking(move || state::read_state(&p.root, &p.git_dir, None)).await?,
                ) {
                    (Some(gh), Ok(st)) => match st.default_remote().and_then(|r| r.forge.clone()) {
                        Some(f) => {
                            let n = card.number;
                            blocking(move || gh.comments(&f, n))
                                .await?
                                .map_err(git_error)?
                        }
                        None => (Vec::new(), None),
                    },
                    _ => {
                        return Err(CoreError::new(
                            ErrorCode::NotFound,
                            "Reading review comments needs gh (signed in); open the pull request on GitHub instead.",
                        ));
                    }
                };
                Ok(pr::review_message(
                    &card,
                    &comments,
                    lines_unread.as_deref(),
                ))
            }
        }
    }
}

impl Core {
    /// "Send failing checks to Vibe" with the failing lines of each failing GitHub Actions check:
    /// gh signed in -> the job's `--log-failed` tail; otherwise the check run's annotations from the public API; when
    /// neither works the message says why for that check. Every quoted line is scrubbed (`checklog::scrub` and the
    /// registered secrets, `vw_secrets::scrub`).
    async fn checks_with_logs(&self, card: &PrCard, place: &Place) -> String {
        let p = place.clone();
        let forge = blocking(move || state::read_state(&p.root, &p.git_dir, None))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|st| st.default_remote().and_then(|r| r.forge.clone()))
            .filter(|f| f.kind == ForgeKind::GitHub);
        let gh = match (self.gh(), &forge) {
            (Some(gh), Some(f)) => {
                let (g, h) = (gh.clone(), f.host.clone());
                let signed = blocking(move || g.signed_in(&h))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(false);
                signed.then_some(gh)
            }
            _ => None,
        };
        let mut logs: Vec<(String, CheckLog)> = Vec::new();
        for item in checklog::failing(card) {
            let log = match (item.url.as_deref().and_then(checklog::job_of), &forge) {
                (None, _) => CheckLog::Unavailable(
                    "it is not a GitHub Actions job (open its page for the details)".to_owned(),
                ),
                (Some(_), None) => CheckLog::Unavailable("the remote is not on GitHub".to_owned()),
                (Some(job), Some(f)) => self.check_log(gh.as_ref(), f, job).await,
            };
            logs.push((item.name.clone(), log));
        }
        let text = checklog::checks_message_with_logs(card, &logs);
        match vw_secrets::scrub(text.as_bytes()) {
            Some((clean, _)) => String::from_utf8_lossy(&clean).into_owned(),
            None => text,
        }
    }

    async fn check_log(
        &self,
        gh: Option<&vw_git::gh::Gh>,
        forge: &vw_git::forge::Forge,
        job: u64,
    ) -> CheckLog {
        if let Some(gh) = gh {
            let (g, f) = (gh.clone(), forge.clone());
            match blocking(move || g.failed_log(&f, job)).await {
                Ok(Ok(raw)) if !raw.trim().is_empty() => {
                    return CheckLog::Lines {
                        source: format!("last lines of `gh run view --job {job} --log-failed`"),
                        text: checklog::tail_of_gh_log(&raw),
                    };
                }
                Ok(Ok(_)) => {
                    return CheckLog::Unavailable(
                        "gh returned no failed steps for this job".to_owned(),
                    );
                }
                Ok(Err(e)) => return CheckLog::Unavailable(format!("gh could not read it ({e})")),
                Err(_) => return CheckLog::Unavailable("gh could not be run".to_owned()),
            }
        }
        match crate::git_api::annotations(&self.shared, forge, job).await {
            Ok(Some(text)) => CheckLog::Lines {
                source: "GitHub's annotations for this check; sign in to gh to include the log itself".to_owned(),
                text,
            },
            Ok(None) => CheckLog::Unavailable(
                "gh is not signed in and GitHub lists no error annotations for it; sign in to gh (`gh auth login`) to \
                 include the failing log"
                    .to_owned(),
            ),
            Err(why) => CheckLog::Unavailable(why),
        }
    }
}
