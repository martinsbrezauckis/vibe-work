//! Durable push jobs and chat notices. One pending record is saved before the network action.
//! Depends on the private app tools folder and the existing transcript notice shape; called by the push tool.
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

const MAX_SETTLED: usize = 500;
const MAX_LEDGER_BYTES: u64 = 4 * 1024 * 1024;
const UNKNOWN: &str = "Outcome unknown: the app restarted; check the remote before another push.";

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    id: String,
    session: String,
    at_ms: u64,
    facts: Value,
    completed_at: Option<u64>,
    state: JobState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
enum JobState {
    Pending,
    Finished(Value),
    Failed(String),
    Unknown,
}

#[derive(Serialize, Deserialize)]
struct Ledger {
    version: u8,
    jobs: VecDeque<Job>,
}

pub(crate) struct Jobs {
    path: PathBuf,
    records: Mutex<VecDeque<Job>>,
    changed: watch::Sender<u64>,
}

impl Jobs {
    pub(crate) fn open(path: PathBuf) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or("Push job file has no parent directory")?;
        vw_platform::ensure_private_dir(parent).map_err(|e| e.to_string())?;
        let mut records = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_LEDGER_BYTES
                {
                    return Err("Push job file is not a usable private file".into());
                }
                if !vw_platform::is_user_only(&path).map_err(|e| e.to_string())? {
                    return Err("Push job file is not private".into());
                }
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if ledger.version != 1 {
                    return Err("Unsupported push job file version".into());
                }
                ledger.jobs
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(error) => return Err(error.to_string()),
        };
        let mut changed_on_open = false;
        for job in &mut records {
            if matches!(job.state, JobState::Pending) {
                job.state = JobState::Unknown;
                job.completed_at = Some(now_ms());
                changed_on_open = true;
            }
        }
        if changed_on_open {
            persist(&path, &records)?;
        }
        let (changed, _) = watch::channel(0);
        Ok(Self {
            path,
            records: Mutex::new(records),
            changed,
        })
    }

    /// Saves one job before the app starts any network side effect.
    pub(crate) fn start(&self, session: &str, at_ms: u64, facts: Value) -> Result<String, String> {
        if session.is_empty() || !facts.is_object() {
            return Err("Push job needs a chat and confirmed plan facts".into());
        }
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        if records
            .iter()
            .any(|job| job.session == session && matches!(job.state, JobState::Pending))
        {
            return Err("A push is already running in this chat. Call push with pushId latest to collect its result.".into());
        }
        let id = vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?;
        let mut next = records.clone();
        while next
            .iter()
            .filter(|job| !matches!(job.state, JobState::Pending))
            .count()
            >= MAX_SETTLED
        {
            if let Some(index) = next
                .iter()
                .position(|job| !matches!(job.state, JobState::Pending))
            {
                next.remove(index);
            }
        }
        next.push_back(Job {
            id: id.clone(),
            session: session.into(),
            at_ms,
            facts,
            completed_at: None,
            state: JobState::Pending,
        });
        persist(&self.path, &next)?;
        *records = next;
        Ok(id)
    }

    /// Saves the exact outcome, then wakes callers collecting this job's result.
    pub(crate) fn finish(&self, id: &str, result: Result<Value, String>) -> Result<(), String> {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = records.clone();
        let job = next
            .iter_mut()
            .find(|job| job.id == id && matches!(job.state, JobState::Pending))
            .ok_or("Push job is missing or already settled")?;
        job.completed_at = Some(now_ms());
        job.state = match result {
            Ok(value) => JobState::Finished(value),
            Err(reason) => JobState::Failed(crate::guard_refusals::clean(&reason)),
        };
        let saved = persist(&self.path, &next);
        // A disk failure after a remote push must not hide a result from the running chat.
        *records = next;
        self.changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        saved
    }

    /// Removes a deleted chat's pending and settled jobs from the private ledger.
    pub(crate) fn forget(&self, session: &str) -> Result<(), String> {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = records.clone();
        next.retain(|job| job.session != session);
        if next.len() == records.len() {
            return Ok(());
        }
        persist(&self.path, &next)?;
        *records = next;
        self.changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }

    /// Moves all saved jobs to the new owner when a chat session is handed off.
    pub(crate) fn hand_off(&self, old: &str, new: &str) -> Result<(), String> {
        if new.is_empty() {
            return Err("A push job cannot move to an empty chat".into());
        }
        if old == new {
            return Ok(());
        }
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = records.clone();
        let mut moved = false;
        for job in &mut next {
            if job.session == old {
                job.session = new.into();
                moved = true;
            }
        }
        if !moved {
            return Ok(());
        }
        persist(&self.path, &next)?;
        *records = next;
        self.changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }

    /// Returns the current owner of a job, including after a chat handoff.
    pub(crate) fn session(&self, id: &str) -> Option<String> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.session.clone())
    }

    /// Reads only a job belonging to `session`; `latest` recovers a lost start reply.
    pub(crate) async fn result(
        &self,
        session: &str,
        id: &str,
        wait: Duration,
    ) -> Result<Value, String> {
        let mut changed = self.changed.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        let id = if id == "latest" {
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            records
                .iter()
                .rev()
                .find(|job| job.session == session)
                .map(|job| job.id.clone())
                .ok_or("No push job for this chat")?
        } else {
            id.to_owned()
        };
        loop {
            let found = {
                let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
                records
                    .iter()
                    .find(|job| job.session == session && job.id == id)
                    .cloned()
            };
            let job = found.ok_or("No push job for this chat")?;
            match job.state {
                JobState::Finished(value) => return Ok(value),
                JobState::Failed(reason) => return Err(reason),
                JobState::Unknown => return Err(UNKNOWN.into()),
                JobState::Pending => {
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        return Ok(json!({
                            "status":"pushing", "pushId":job.id, "facts":job.facts,
                            "note":"Call push with this pushId to collect the result; do not start another push."
                        }));
                    }
                    let _ = tokio::time::timeout_at(deadline, changed.changed()).await;
                }
            }
        }
    }

    /// Adds durable success and uncertain-outcome notices to one chat snapshot.
    pub(crate) fn overlay(&self, session: &str, state: &mut Value) {
        let Some(history) = state.get_mut("history").and_then(Value::as_array_mut) else {
            return;
        };
        history.retain(|entry| entry.get("vwPushNotice") != Some(&Value::Bool(true)));
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        for job in records.iter().filter(|job| job.session == session) {
            let (message, level, status) = match &job.state {
                JobState::Finished(value)
                    if value.get("status").and_then(Value::as_str) == Some("pushed") =>
                {
                    let mut message = value
                        .get("message")
                        .and_then(Value::as_str)
                        .map_or_else(|| push_message(value), str::to_owned);
                    let warning =
                        value
                            .get("summary")
                            .and_then(Value::as_str)
                            .and_then(|summary| {
                                summary
                                    .find("The push succeeded, but")
                                    .map(|at| &summary[at..])
                            });
                    if let Some(warning) = warning {
                        message.push(' ');
                        message.push_str(&crate::guard_refusals::clean(warning));
                    }
                    (
                        message,
                        if warning.is_some() { "warning" } else { "info" },
                        "completed",
                    )
                }
                JobState::Unknown => (
                    format!(
                        "{UNKNOWN} Attempted {} to {}/{} at {}.",
                        fact(&job.facts, "branch"),
                        fact(&job.facts, "remote"),
                        fact(&job.facts, "remoteBranch"),
                        fact(&job.facts, "commit")
                    ),
                    "warning",
                    "unknown",
                ),
                _ => continue,
            };
            let created_at = job.completed_at.unwrap_or(job.at_ms);
            let entry = json!({
                "id":format!("vw-push-{}", job.id), "sessionId":session, "type":"notice",
                "createdAt":created_at, "vwPushNotice":true, "level":level,
                "message":message, "detail":{"kind":"git_push",
                    "remote":job.facts.get("remote"), "address":job.facts.get("address"),
                    "branch":job.facts.get("remoteBranch"), "commit":job.facts.get("commit"),
                    "commits":job.facts.get("commits")},
                "state":{"status":status}
            });
            let next = history
                .iter()
                .position(|entry| entry["createdAt"].as_u64() > Some(created_at));
            history.insert(next.unwrap_or(history.len()), entry);
        }
    }
}

fn fact<'a>(facts: &'a Value, key: &str) -> &'a str {
    facts.get(key).and_then(Value::as_str).unwrap_or("unknown")
}

fn push_message(value: &Value) -> String {
    format!(
        "Pushed {} to {}/{}. Commit: {}.",
        fact(value, "remoteBranch"),
        fact(value, "remote"),
        fact(value, "remoteBranch"),
        fact(value, "commit")
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn persist(path: &Path, records: &VecDeque<Job>) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Ledger {
        version: 1,
        jobs: records.clone(),
    })
    .map_err(|e| e.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_LEDGER_BYTES {
        return Err("Push job file is full".into());
    }
    vw_platform::replace_private_durable(path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open() -> (tempfile::TempDir, Jobs) {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::open(dir.path().join("push-jobs.json")).unwrap();
        (dir, jobs)
    }

    fn facts() -> Value {
        json!({"remote":"origin","address":"/tmp/local.git","branch":"topic", "remoteBranch":"topic","commit":"abc123","commits":1})
    }

    // PREDICT: a completed push keeps its exact result and notice after restart, scoped to its chat.
    #[tokio::test]
    async fn finished_result_and_notice_survive_restart_and_stay_in_own_chat() {
        let (dir, jobs) = open();
        let id = jobs.start("chat-one", 12, facts()).unwrap();
        jobs.finish(&id, Ok(json!({"status":"pushed", "remote":"origin", "remoteBranch":"topic", "commit":"abc123", "message":"Pushed topic to origin/topic. Commit: abc123.", "summary":"Pushed topic to origin (/tmp/local.git). The push succeeded, but the local upstream was not set: config locked."}))).unwrap();
        drop(jobs);
        let jobs = Jobs::open(dir.path().join("push-jobs.json")).unwrap();
        assert_eq!(
            jobs.result("chat-one", &id, Duration::ZERO).await.unwrap()["commit"],
            "abc123"
        );
        assert!(jobs.result("chat-two", &id, Duration::ZERO).await.is_err());
        let mut own = json!({"history":[]});
        let mut other = json!({"history":[]});
        jobs.overlay("chat-one", &mut own);
        jobs.overlay("chat-one", &mut own);
        jobs.overlay("chat-two", &mut other);
        assert_eq!(own["history"].as_array().unwrap().len(), 1);
        assert_eq!(own["history"][0]["type"], "notice");
        assert!(
            own["history"][0]["message"]
                .as_str()
                .unwrap()
                .contains("the local upstream was not set")
        );
        assert_eq!(own["history"][0]["level"], "warning");
        assert_eq!(other["history"], json!([]));
    }

    #[tokio::test]
    async fn pending_becomes_unknown_after_restart_and_never_retries() {
        let (dir, jobs) = open();
        let id = jobs.start("chat", 22, facts()).unwrap();
        drop(jobs);
        let jobs = Jobs::open(dir.path().join("push-jobs.json")).unwrap();
        let reason = jobs.result("chat", &id, Duration::ZERO).await.unwrap_err();
        assert!(reason.starts_with(vw_agent_tools::wire::UNKNOWN_OUTCOME));
        assert!(reason.contains("Outcome unknown"), "{reason}");
        assert!(reason.contains("check the remote"), "{reason}");
        let mut history = json!({"history":[]});
        let mut other = json!({"history":[]});
        jobs.overlay("chat", &mut history);
        jobs.overlay("other", &mut other);
        assert_eq!(history["history"][0]["type"], "notice");
        assert_eq!(history["history"][0]["state"]["status"], "unknown");
        assert!(
            history["history"][0]["message"]
                .as_str()
                .unwrap()
                .contains("Outcome unknown")
        );
        assert!(
            history["history"][0]["message"]
                .as_str()
                .unwrap()
                .contains("abc123")
        );
        assert_eq!(other["history"], json!([]));
        assert!(
            jobs.start("chat", 23, facts()).is_ok(),
            "an unknown job cannot silently retry"
        );
    }

    #[tokio::test]
    async fn pending_returns_one_id_and_settles_for_that_chat_only() {
        let (_dir, jobs) = open();
        let id = jobs.start("chat", 33, facts()).unwrap();
        assert!(
            jobs.start("chat", 34, facts()).is_err(),
            "two starts could push twice"
        );
        let pending = jobs.result("chat", &id, Duration::ZERO).await.unwrap();
        assert_eq!(pending["status"], "pushing");
        assert_eq!(pending["pushId"], id);
        assert_eq!(pending["facts"]["remoteBranch"], "topic");
        assert!(jobs.result("other", &id, Duration::ZERO).await.is_err());
        jobs.finish(&id, Err("non-fast-forward".into())).unwrap();
        assert_eq!(
            jobs.result("chat", &id, Duration::ZERO).await.unwrap_err(),
            "non-fast-forward"
        );
    }

    #[tokio::test]
    async fn latest_recovers_a_lost_reply_without_cross_chat_access() {
        let (_dir, jobs) = open();
        let first = jobs.start("chat-one", 40, facts()).unwrap();
        jobs.finish(
            &first,
            Ok(json!({"status":"pushed", "remote":"origin", "remoteBranch":"old", "commit":"old"})),
        )
        .unwrap();
        let second = jobs.start("chat-one", 41, facts()).unwrap();
        let pending = jobs
            .result("chat-one", "latest", Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(pending["pushId"], second);
        assert!(
            jobs.result("chat-two", "latest", Duration::ZERO)
                .await
                .is_err()
        );
        jobs.finish(
            &second,
            Ok(json!({"status":"pushed", "remote":"origin", "remoteBranch":"new", "commit":"new"})),
        )
        .unwrap();
        assert_eq!(
            jobs.result("chat-one", "latest", Duration::ZERO)
                .await
                .unwrap()["commit"],
            "new"
        );
    }

    #[tokio::test]
    async fn waiter_receives_result_when_push_finishes_before_timeout() {
        let (_dir, jobs) = open();
        let jobs = std::sync::Arc::new(jobs);
        let id = jobs.start("chat", 50, facts()).unwrap();
        let waiter = {
            let jobs = jobs.clone();
            let id = id.clone();
            tokio::spawn(async move { jobs.result("chat", &id, Duration::from_secs(2)).await })
        };
        tokio::task::yield_now().await;
        jobs.finish(&id, Ok(json!({"status":"pushed", "remote":"origin", "remoteBranch":"main", "commit":"tip"}))).unwrap();
        let answer = waiter.await.unwrap().unwrap();
        assert_eq!(answer["commit"], "tip");
    }

    // PREDICT: chat deletion removes durable push jobs and notices, including after restart.
    #[tokio::test]
    async fn deleted_chat_forgets_jobs_and_notices_durably() {
        let (dir, jobs) = open();
        let id = jobs.start("deleted", 60, facts()).unwrap();
        jobs.finish(&id, Ok(json!({"status":"pushed","remote":"origin","remoteBranch":"topic","commit":"abc123"}))).unwrap();
        let keep = jobs.start("kept", 61, facts()).unwrap();
        jobs.forget("deleted").unwrap();
        assert_eq!(jobs.session(&id), None);
        drop(jobs);
        let jobs = Jobs::open(dir.path().join("push-jobs.json")).unwrap();
        assert!(
            jobs.result("deleted", "latest", Duration::ZERO)
                .await
                .is_err()
        );
        assert_eq!(jobs.session(&keep).as_deref(), Some("kept"));
        let mut state = json!({"history":[]});
        jobs.overlay("deleted", &mut state);
        assert_eq!(state["history"], json!([]));
    }

    // PREDICT: a handoff moves the pending result and future notice to the new chat only.
    #[tokio::test]
    async fn handoff_moves_pending_job_and_notice_to_new_chat_durably() {
        let (dir, jobs) = open();
        let id = jobs.start("old", 70, facts()).unwrap();
        jobs.hand_off("old", "new").unwrap();
        assert_eq!(jobs.session(&id).as_deref(), Some("new"));
        assert!(jobs.result("old", &id, Duration::ZERO).await.is_err());
        jobs.finish(&id, Ok(json!({"status":"pushed","remote":"origin","remoteBranch":"topic","commit":"abc123"}))).unwrap();
        drop(jobs);
        let jobs = Jobs::open(dir.path().join("push-jobs.json")).unwrap();
        assert_eq!(
            jobs.result("new", &id, Duration::ZERO).await.unwrap()["commit"],
            "abc123"
        );
        let mut old = json!({"history":[]});
        let mut new = json!({"history":[]});
        jobs.overlay("old", &mut old);
        jobs.overlay("new", &mut new);
        assert_eq!(old["history"], json!([]));
        assert_eq!(new["history"].as_array().unwrap().len(), 1);
    }
}
