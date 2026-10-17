//! Selected Vibe 2.26.1 fields; serde skips content/config secrets rather than retaining payloads.
//! Scanner reads these types directly from buffered streams; optional counters never default to zero.

use crate::{
    Error, LastModelCallSnapshot, ModelSnapshot, RecordedToolOutcomes, SessionStats, TokenUsage,
    ToolUsage,
};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde::de::{MapAccess, SeqAccess, Visitor};

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    #[serde(default)]
    pub stats: Option<Stats>,
    #[serde(default)]
    pub config: Option<Config>,
    #[serde(default)]
    pub child_sessions: Vec<ChildLink>,
}

#[derive(Default, Deserialize)]
pub(crate) struct Stats {
    session_prompt_tokens: Option<u64>,
    session_completion_tokens: Option<u64>,
    session_cached_tokens: Option<u64>,
    last_turn_prompt_tokens: Option<u64>,
    last_turn_completion_tokens: Option<u64>,
    last_turn_cached_tokens: Option<u64>,
    last_turn_duration: Option<f64>,
    steps: Option<u64>,
    tool_calls_agreed: Option<u64>,
    tool_calls_rejected: Option<u64>,
    tool_calls_hook_denied: Option<u64>,
    tool_calls_failed: Option<u64>,
    tool_calls_succeeded: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct Config {
    active_model: Option<String>,
    #[serde(default)]
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    alias: Option<String>,
    name: Option<String>,
    provider: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ChildLink {
    pub session_id: String,
    pub tool_call_id: String,
    pub agent: String,
    pub relative_path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Deserialize)]
pub(crate) struct Message {
    pub role: Role,
    #[serde(rename = "message_id")]
    pub id: Option<String>,
    #[serde(default)]
    pub injected: bool,
    pub manual_shell: Option<IgnoredAny>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub tool_result: Option<ToolResult>,
}

#[derive(Deserialize)]
pub(crate) struct ToolCall {
    pub id: Option<String>,
    #[serde(default)]
    pub function: Function,
}

#[derive(Default, Deserialize)]
pub(crate) struct Function {
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ToolResult {
    pub duration: Option<f64>,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub output: TaskOutput,
}

#[derive(Default, Deserialize)]
pub(crate) struct TaskOutput {
    pub turns_used: Option<TaskMetric>,
    pub completed: Option<TaskMetric>,
}

// Tool output is an arbitrary JSON object. Only scalar task metrics are retained, so an unrelated
// plugin's identically named fields neither reject its valid output nor retain its nested payload.
pub(crate) enum TaskMetric {
    Count(u64),
    Flag(bool),
    Unknown,
}

impl TaskMetric {
    pub fn count(self) -> Option<u64> {
        match self {
            Self::Count(n) => Some(n),
            _ => None,
        }
    }

    pub fn flag(self) -> Option<bool> {
        match self {
            Self::Flag(flag) => Some(flag),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for TaskMetric {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MetricVisitor;
        impl<'de> Visitor<'de> for MetricVisitor {
            type Value = TaskMetric;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a task metric or unrelated tool output")
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(TaskMetric::Count(value))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(u64::try_from(value).map_or(TaskMetric::Unknown, TaskMetric::Count))
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(TaskMetric::Flag(value))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(TaskMetric::Unknown)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(TaskMetric::Unknown)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(TaskMetric::Unknown)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                while sequence.next_element::<IgnoredAny>()?.is_some() {}
                Ok(TaskMetric::Unknown)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(TaskMetric::Unknown)
            }
        }
        deserializer.deserialize_any(MetricVisitor)
    }
}

pub(crate) fn duration(value: Option<f64>) -> Result<Option<f64>, Error> {
    if value.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(Error::InvalidDuration);
    }
    Ok(value)
}

impl Metadata {
    pub fn summary(&mut self) -> Result<SessionStats, Error> {
        let stats = self.stats.take().unwrap_or_default();
        let usage = TokenUsage {
            input: stats.session_prompt_tokens,
            output: stats.session_completion_tokens,
            cached: stats.session_cached_tokens,
        }
        .validate()?;
        let last_model_call = LastModelCallSnapshot {
            usage: TokenUsage {
                input: stats.last_turn_prompt_tokens,
                output: stats.last_turn_completion_tokens,
                cached: stats.last_turn_cached_tokens,
            }
            .validate()?,
            duration_seconds: duration(stats.last_turn_duration)?,
        };
        let mut model_snapshot = ModelSnapshot::default();
        if let Some(config) = self.config.take() {
            if let Some(model) = config
                .models
                .into_iter()
                .find(|m| m.alias.is_some() && m.alias == config.active_model)
            {
                model_snapshot.name = model.name;
                model_snapshot.provider = model.provider;
            }
            model_snapshot.alias = config.active_model;
        }
        Ok(SessionStats {
            session_id: self.session_id.clone(),
            parent_session_id: self.parent_session_id.clone(),
            start_time: self.start_time.take(),
            last_saved_at: self.end_time.take(),
            model: None,
            model_snapshot,
            usage,
            last_model_call,
            recorded_steps: stats.steps,
            recorded_tools: RecordedToolOutcomes {
                agreed: stats.tool_calls_agreed,
                rejected: stats.tool_calls_rejected,
                hook_denied: stats.tool_calls_hook_denied,
                failed: stats.tool_calls_failed,
                succeeded: stats.tool_calls_succeeded,
            },
            turns: 0,
            messages: 0,
            tools: ToolUsage::default(),
            unattributed_tool_calls: 0,
            tool_seconds: Some(0.0),
            model_seconds: None,
            wall_seconds: None,
            cost: None,
            subagent_sessions: 0,
            attributed_usage: usage,
            attributed_cost: None,
        })
    }
}
