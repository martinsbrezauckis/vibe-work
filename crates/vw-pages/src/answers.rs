//! Answers to page inputs (Handbook Format 1.2 §6.6): what the user picked, ticked, ranked or wrote for an
//! input of a handbook page, kept in Vibe Work's own data like comments, never in the repository. Keyed by project
//! folder + page path + input id; an answer whose input (or one of its options) is gone is listed as detached, never
//! dropped. The agent reads them back with the `handbook_answers` tool; "Send to Vibe" puts all answers of a page
//! into the composer as one context chip ([`PageAnswers::text`]).
//!
//! Depends on: serde, vw-platform (user-only files), `comments` (project key, clock), `page` (inputs), `paths`.
//! Called by: `service`, vw-core-api (the `pages` answer requests, the agent tool), tests.
//! Invariants:
//! - An answer is checked against the input in the page FILE when it is set: the type must match and every option
//!   must exist (a rank names every option exactly once); a client cannot store an option the page does not have.
//! - One JSON file per project (`<dir>/<project key>.json`), user-only, replaced atomically; a file that does not
//!   parse is moved aside and the project starts with no answers.
//! - Text answers at most [`MAX_TEXT`] characters; at most [`MAX_PER_PROJECT`] answers.
//! - Every label in [`PageAnswers`] comes from the page file (untrusted page text, capped by the page reader).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::comments::project_key;
use crate::page::{Input, PageFacts};
use crate::paths::PagePath;

/// Longest text answer (§6.6).
pub const MAX_TEXT: usize = 2000;
/// Most answers per project.
pub const MAX_PER_PROJECT: usize = 5000;

/// What the user answered, by input type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AnswerValue {
    /// A `choice` input: the option picked.
    Choice {
        /// The option id.
        option: String,
    },
    /// A `checklist` input: the options ticked, in page order.
    Checklist {
        /// Option ids.
        checked: Vec<String>,
    },
    /// A `rank` input: every option, most important first.
    Rank {
        /// Option ids.
        order: Vec<String>,
    },
    /// A `text` input.
    Text {
        /// The answer.
        text: String,
    },
}

impl AnswerValue {
    /// The input type this value answers.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Choice { .. } => "choice",
            Self::Checklist { .. } => "checklist",
            Self::Rank { .. } => "rank",
            Self::Text { .. } => "text",
        }
    }
}

/// One stored answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// The page (path inside the handbook folder).
    pub page: String,
    /// The input id.
    pub input: String,
    /// The question when the answer was given (shown for a detached answer).
    pub question: String,
    /// The answer.
    pub value: AnswerValue,
    /// Last changed, ms since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated_ms: u64,
    /// Put into the composer at, ms (`None`: not sent since the last change).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub sent_ms: Option<u64>,
}

/// One input of a page with the user's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InputAnswer {
    /// The input as the page file declares it.
    pub input: Input,
    /// The answer (`None`: not answered yet).
    pub answer: Option<Answer>,
    /// The answer in words: the chosen, ticked or ranked option labels in order, or the text.
    pub words: Vec<String>,
}

/// Every input of one page with its answers, plus answers whose input is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PageAnswers {
    /// The page.
    pub page: String,
    /// Its title.
    pub title: Option<String>,
    /// Its inputs in page order.
    pub inputs: Vec<InputAnswer>,
    /// Answers whose input is gone or no longer fits (type changed, an option removed).
    pub detached: Vec<Answer>,
    /// Inputs answered.
    pub answered: u32,
    /// Answered inputs not sent since their last change.
    pub unsent: u32,
}

impl PageAnswers {
    /// The answers as plain text for the agent ("Send to Vibe" chip): the page, then one line per input. `folder` names
    /// the handbook folder the page is in (`docs/handbook`). Page text is quoted, never a command.
    #[must_use]
    pub fn text(&self, folder: &str) -> String {
        let mut out = format!(
            "Answers to the inputs on the page {folder}/{} (\"{}\"):\n",
            self.page,
            self.title.as_deref().unwrap_or(&self.page)
        );
        for item in &self.inputs {
            let question = if item.input.question.is_empty() {
                item.input.id.as_str()
            } else {
                item.input.question.as_str()
            };
            let answer = match (&item.answer, item.input.kind.as_str()) {
                (None, _) => "(not answered)".to_owned(),
                (Some(_), "rank") => item
                    .words
                    .iter()
                    .enumerate()
                    .map(|(n, w)| format!("{}. {w}", n + 1))
                    .collect::<Vec<_>>()
                    .join("; "),
                (Some(_), "text") => format!("\"{}\"", item.words.join(" ")),
                (Some(_), _) if item.words.is_empty() => "(none ticked)".to_owned(),
                (Some(_), _) => item.words.join("; "),
            };
            let _ = writeln!(out, "- #{} {question}: {answer}", item.input.id);
        }
        for gone in &self.detached {
            let _ = writeln!(
                out,
                "- #{} (no longer on the page) {}: {}",
                gone.input,
                gone.question,
                value_words(&gone.value).join("; ")
            );
        }
        out.push_str(
            "Read them any time with handbook_answers; record the outcome in the page itself.",
        );
        out
    }
}

/// Why an answer operation failed.
#[derive(Debug, thiserror::Error)]
pub enum AnswerError {
    /// The value does not fit the input, or the input does not exist.
    #[error("{0}")]
    Invalid(String),
    /// The store file could not be read or written.
    #[error("the answers could not be saved: {0}")]
    Io(String),
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    version: u32,
    answers: Vec<Answer>,
}

/// Checks `value` against `input` (the page file's declaration). `Ok` with the value normalized (checklist in page
/// order, text trimmed).
///
/// # Errors
/// A plain-words reason.
pub fn fit(input: &Input, value: &AnswerValue) -> Result<AnswerValue, String> {
    if input.kind != value.kind() {
        return Err(format!(
            "input {} is a {} input, not {}",
            input.id,
            input.kind,
            value.kind()
        ));
    }
    let known = |id: &String| input.options.iter().any(|o| o.id == *id);
    match value {
        AnswerValue::Choice { option } if known(option) => Ok(value.clone()),
        AnswerValue::Choice { option } => Err(format!(
            "input {} has no option \"{}\"",
            input.id,
            crate::html::clip(option, 40)
        )),
        AnswerValue::Checklist { checked } => {
            if let Some(bad) = checked.iter().find(|c| !known(c)) {
                return Err(format!(
                    "input {} has no option \"{}\"",
                    input.id,
                    crate::html::clip(bad, 40)
                ));
            }
            Ok(AnswerValue::Checklist {
                checked: input
                    .options
                    .iter()
                    .filter(|o| checked.contains(&o.id))
                    .map(|o| o.id.clone())
                    .collect(),
            })
        }
        AnswerValue::Rank { order } => {
            let mut sorted = order.clone();
            sorted.sort();
            sorted.dedup();
            let all = sorted.len() == order.len()
                && order.len() == input.options.len()
                && order.iter().all(known);
            if all {
                Ok(value.clone())
            } else {
                Err(format!(
                    "a rank of input {} names every option exactly once ({})",
                    input.id,
                    input
                        .options
                        .iter()
                        .map(|o| o.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
        AnswerValue::Text { text } => {
            let text = text.trim();
            if text.is_empty() {
                Err("the answer is empty (clear it instead)".to_owned())
            } else if text.chars().count() > MAX_TEXT {
                Err(format!("a text answer has at most {MAX_TEXT} characters"))
            } else {
                Ok(AnswerValue::Text {
                    text: text.to_owned(),
                })
            }
        }
    }
}

/// The option labels (or the text) of `value` as the page file names them; ids without a label stay ids.
fn words(input: Option<&Input>, value: &AnswerValue) -> Vec<String> {
    let label = |id: &String| {
        input
            .and_then(|i| i.options.iter().find(|o| o.id == *id))
            .map_or_else(|| id.clone(), |o| o.label.clone())
    };
    match value {
        AnswerValue::Choice { option } => vec![label(option)],
        AnswerValue::Checklist { checked } => checked.iter().map(label).collect(),
        AnswerValue::Rank { order } => order.iter().map(label).collect(),
        AnswerValue::Text { text } => vec![text.clone()],
    }
}

fn value_words(value: &AnswerValue) -> Vec<String> {
    words(None, value)
}

/// The answers of every project, in `dir`.
pub struct AnswerStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl AnswerStore {
    /// A store in `dir` (created user-only on first write).
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    fn file(&self, project: &Path) -> PathBuf {
        self.dir.join(format!("{}.json", project_key(project)))
    }

    fn load(&self, project: &Path) -> Result<File, AnswerError> {
        let path = self.file(project);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(File::default());
            }
            Err(error) => return Err(AnswerError::Io(error.to_string())),
        };
        if let Ok(file) = serde_json::from_str::<File>(&text) {
            return Ok(file);
        }
        let aside = vw_platform::set_aside(&path, now_ms()).ok_or_else(|| {
            AnswerError::Io("the unreadable answers file could not be moved aside".to_owned())
        })?;
        tracing::warn!(file = ?aside, "page answers file did not parse: moved aside");
        Ok(File::default())
    }

    fn save(&self, project: &Path, file: &File) -> Result<(), AnswerError> {
        vw_platform::ensure_private_dir(&self.dir).map_err(|e| AnswerError::Io(e.to_string()))?;
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| AnswerError::Io(e.to_string()))?;
        vw_platform::replace_private(&self.file(project), &bytes)
            .map_err(|e| AnswerError::Io(e.to_string()))
    }

    /// Every stored answer of `project` (of `page` only, when given), oldest first.
    #[must_use]
    pub fn list(&self, project: &Path, page: Option<&PagePath>) -> Vec<Answer> {
        let _guard = self.lock.lock();
        self.load(project)
            .unwrap_or_default()
            .answers
            .into_iter()
            .filter(|a| page.is_none_or(|p| a.page == p.as_str()))
            .collect()
    }

    /// Sets the answer to input `input_id` of `page`, checked against `facts` (the page file). Replaces an earlier
    /// answer to the same input and clears its sent mark.
    ///
    /// # Errors
    /// [`AnswerError::Invalid`] (no such input, a value that does not fit, a full store); [`AnswerError::Io`].
    pub fn set(
        &self,
        project: &Path,
        page: &PagePath,
        facts: &PageFacts,
        input_id: &str,
        value: &AnswerValue,
    ) -> Result<Answer, AnswerError> {
        let input = facts
            .inputs
            .iter()
            .find(|i| i.id == input_id)
            .ok_or_else(|| {
                AnswerError::Invalid(format!(
                    "the page has no input \"{}\"",
                    crate::html::clip(input_id, 60)
                ))
            })?;
        let value = fit(input, value).map_err(AnswerError::Invalid)?;
        let _guard = self.lock.lock();
        let mut file = self.load(project)?;
        file.answers
            .retain(|a| !(a.page == page.as_str() && a.input == input.id));
        if file.answers.len() >= MAX_PER_PROJECT {
            return Err(AnswerError::Invalid(
                "this project has too many answers; clear some first".to_owned(),
            ));
        }
        let answer = Answer {
            page: page.to_string(),
            input: input.id.clone(),
            question: input.question.clone(),
            value,
            updated_ms: now_ms(),
            sent_ms: None,
        };
        file.version = 1;
        file.answers.push(answer.clone());
        self.save(project, &file)?;
        Ok(answer)
    }

    /// Removes the answer to `input_id` of `page` (also a detached one). Returns whether one existed.
    ///
    /// # Errors
    /// [`AnswerError::Io`].
    pub fn clear(
        &self,
        project: &Path,
        page: &PagePath,
        input_id: &str,
    ) -> Result<bool, AnswerError> {
        let _guard = self.lock.lock();
        let mut file = self.load(project)?;
        let before = file.answers.len();
        file.answers
            .retain(|a| !(a.page == page.as_str() && a.input == input_id));
        if file.answers.len() == before {
            return Ok(false);
        }
        self.save(project, &file)?;
        Ok(true)
    }

    /// Marks every answer of `page` as put into the composer now. Returns how many.
    ///
    /// # Errors
    /// [`AnswerError::Invalid`] when the page has no answers; [`AnswerError::Io`].
    pub fn mark_sent(&self, project: &Path, page: &PagePath) -> Result<u32, AnswerError> {
        let _guard = self.lock.lock();
        let mut file = self.load(project)?;
        let now = now_ms();
        let mut marked = 0u32;
        for answer in file.answers.iter_mut().filter(|a| a.page == page.as_str()) {
            answer.sent_ms = Some(now);
            marked += 1;
        }
        if marked == 0 {
            return Err(AnswerError::Invalid(
                "this page has no answers to send".to_owned(),
            ));
        }
        self.save(project, &file)?;
        Ok(marked)
    }

    /// Pages of `project` with stored answers (attached or not).
    #[must_use]
    pub fn pages(&self, project: &Path) -> Vec<String> {
        let mut pages: Vec<String> = self
            .list(project, None)
            .into_iter()
            .map(|a| a.page)
            .collect();
        pages.sort();
        pages.dedup();
        pages
    }
}

/// Joins `facts` (the page file; `None`: the page is gone) with the stored `answers` of that page.
#[must_use]
pub fn page_answers(
    page: &PagePath,
    facts: Option<&PageFacts>,
    answers: Vec<Answer>,
) -> PageAnswers {
    let inputs = facts.map_or(&[][..], |f| f.inputs.as_slice());
    let mut detached = Vec::new();
    let mut matched: Vec<Option<Answer>> = vec![None; inputs.len()];
    for answer in answers {
        let slot = inputs
            .iter()
            .position(|i| i.id == answer.input && fit(i, &answer.value).is_ok());
        match slot {
            Some(n) => matched[n] = Some(answer),
            None => detached.push(answer),
        }
    }
    let items: Vec<InputAnswer> = inputs
        .iter()
        .zip(matched)
        .map(|(input, answer)| InputAnswer {
            words: answer
                .as_ref()
                .map(|a| words(Some(input), &a.value))
                .unwrap_or_default(),
            input: input.clone(),
            answer,
        })
        .collect();
    let count = |f: &dyn Fn(&Answer) -> bool| {
        u32::try_from(
            items
                .iter()
                .filter(|i| i.answer.as_ref().is_some_and(f))
                .count(),
        )
        .unwrap_or(u32::MAX)
    };
    PageAnswers {
        page: page.to_string(),
        title: facts.and_then(|f| f.title.clone()),
        answered: count(&|_| true),
        unsent: count(&|a| a.sent_ms.is_none()),
        inputs: items,
        detached,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
