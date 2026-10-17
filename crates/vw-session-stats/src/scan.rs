//! Streaming session aggregation; standard readers and serde feed the public turn callback.
//! Retains selected metadata and current tool state, never transcript text or completed turns.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, ErrorKind, Read};
use std::path::{Component, Path};

use crate::stored::{ChildLink, Message, Metadata, Role, ToolCall, ToolResult, duration};
use crate::{
    Error, ParentTurn, PriceTable, SessionStats, SubagentStats, TokenUsage, ToolCounts, ToolUsage,
    TurnStats,
};

pub(crate) fn directory(
    path: &Path,
    prices: Option<&PriceTable>,
    mut on_turn: impl FnMut(TurnStats),
) -> Result<SessionStats, Error> {
    let metadata = parse_metadata(File::open(path.join("meta.json"))?)?;
    run(
        metadata,
        File::open(path.join("messages.jsonl"))?,
        Some(path),
        prices,
        None,
        &mut on_turn,
    )
}

pub(crate) fn readers(
    metadata: impl Read,
    messages: impl Read,
    prices: Option<&PriceTable>,
    mut on_turn: impl FnMut(TurnStats),
) -> Result<SessionStats, Error> {
    run(
        parse_metadata(metadata)?,
        messages,
        None,
        prices,
        None,
        &mut on_turn,
    )
}

fn parse_metadata(reader: impl Read) -> Result<Metadata, Error> {
    Ok(serde_json::from_reader(BufReader::new(reader))?)
}

fn increment(count: &mut u64) -> Result<(), Error> {
    *count = count.checked_add(1).ok_or(Error::Overflow)?;
    Ok(())
}

fn run(
    mut metadata: Metadata,
    messages: impl Read,
    directory: Option<&Path>,
    prices: Option<&PriceTable>,
    parent: Option<ParentTurn>,
    on_turn: &mut dyn FnMut(TurnStats),
) -> Result<SessionStats, Error> {
    let mut summary = metadata.summary()?;
    summary.cost = prices.and_then(|table| table.cost(None, summary.usage));
    summary.attributed_cost = summary.cost;
    let mut links = HashMap::new();
    for link in metadata.child_sessions {
        if links.insert(link.tool_call_id.clone(), link).is_some() {
            return Err(Error::DuplicateChildLink);
        }
    }
    let mut state = State {
        summary,
        current: None,
        pending: HashMap::new(),
        links,
        directory,
        prices,
        parent,
    };
    let messages =
        serde_json::Deserializer::from_reader(BufReader::new(messages)).into_iter::<Message>();
    for message in messages {
        state.message(message?, on_turn)?;
    }
    state.finish_turn(on_turn);
    state.summary.tool_seconds = state.summary.tools.total.duration_seconds();
    Ok(state.summary)
}

struct PendingCall {
    name: Option<String>,
    child_index: Option<usize>,
}

struct State<'a> {
    summary: SessionStats,
    current: Option<TurnStats>,
    pending: HashMap<String, PendingCall>,
    links: HashMap<String, ChildLink>,
    directory: Option<&'a Path>,
    prices: Option<&'a PriceTable>,
    parent: Option<ParentTurn>,
}

impl State<'_> {
    fn message(
        &mut self,
        message: Message,
        on_turn: &mut dyn FnMut(TurnStats),
    ) -> Result<(), Error> {
        increment(&mut self.summary.messages)?;
        match message.role {
            Role::User if !message.injected && message.manual_shell.is_none() => {
                self.finish_turn(on_turn);
                self.pending.clear();
                increment(&mut self.summary.turns)?;
                self.current = Some(TurnStats {
                    session_id: self.summary.session_id.clone(),
                    index: self.summary.turns,
                    message_id: message.id,
                    parent: self.parent.clone(),
                    model: None,
                    usage: TokenUsage::default(),
                    wall_seconds: None,
                    tool_seconds: Some(0.0),
                    model_seconds: None,
                    succeeded: None,
                    cost: None,
                    tools: ToolUsage::default(),
                    subagents: Vec::new(),
                });
            }
            Role::Assistant => {
                // A new model response ends the preceding tool batch; missing results stay unknown.
                self.pending.clear();
                for call in message.tool_calls.unwrap_or_default() {
                    self.call(call, on_turn)?;
                }
            }
            Role::Tool => self.result(message.tool_call_id, message.tool_result)?,
            Role::System | Role::User => {}
        }
        Ok(())
    }

    fn finish_turn(&mut self, on_turn: &mut dyn FnMut(TurnStats)) {
        if let Some(mut turn) = self.current.take() {
            turn.tool_seconds = turn.tools.total.duration_seconds();
            on_turn(turn);
        }
    }

    fn record(&mut self, name: Option<&str>, counts: &ToolCounts) -> Result<(), Error> {
        self.summary.tools.record(name, counts)?;
        if let Some(turn) = &mut self.current {
            turn.tools.record(name, counts)?;
        }
        Ok(())
    }

    fn call(&mut self, call: ToolCall, on_turn: &mut dyn FnMut(TurnStats)) -> Result<(), Error> {
        self.record(
            call.function.name.as_deref(),
            &ToolCounts {
                calls: 1,
                ..ToolCounts::default()
            },
        )?;
        if self.current.is_none() {
            increment(&mut self.summary.unattributed_tool_calls)?;
        }
        let Some(id) = call.id else {
            return Ok(());
        };
        let child_index = if let Some(link) = self.links.remove(&id) {
            let index = self.child(link, on_turn)?;
            self.current.is_some().then_some(index)
        } else {
            None
        };
        if self
            .pending
            .insert(
                id,
                PendingCall {
                    name: call.function.name,
                    child_index,
                },
            )
            .is_some()
        {
            return Err(Error::DuplicateToolCallId);
        }
        Ok(())
    }

    fn result(&mut self, id: Option<String>, result: Option<ToolResult>) -> Result<(), Error> {
        let seconds = duration(result.as_ref().and_then(|r| r.duration))?;
        let pending = id.and_then(|id| self.pending.remove(&id));
        let Some(pending) = pending else {
            increment(&mut self.summary.tools.unmatched_results)?;
            if let Some(turn) = &mut self.current {
                increment(&mut turn.tools.unmatched_results)?;
            }
            return Ok(());
        };
        self.record(
            pending.name.as_deref(),
            &ToolCounts {
                results: 1,
                timed_calls: u64::from(seconds.is_some()),
                recorded_seconds: seconds.unwrap_or(0.0),
                cancelled: u64::from(result.as_ref().is_some_and(|r| r.cancelled)),
                ..ToolCounts::default()
            },
        )?;
        if let (Some(turn), Some(index), Some(result)) =
            (&mut self.current, pending.child_index, result)
        {
            let child = &mut turn.subagents[index];
            child.turns_used = result
                .output
                .turns_used
                .and_then(crate::stored::TaskMetric::count);
            child.completed = result
                .output
                .completed
                .and_then(crate::stored::TaskMetric::flag);
        }
        Ok(())
    }

    fn child(
        &mut self,
        link: ChildLink,
        on_turn: &mut dyn FnMut(TurnStats),
    ) -> Result<usize, Error> {
        let parent = ParentTurn {
            session_id: self.summary.session_id.clone(),
            turn_index: self.current.as_ref().map(|t| t.index),
            tool_call_id: link.tool_call_id.clone(),
        };
        // Vibe permits one subagent depth. Child scans never follow further links.
        let child = read_child(
            self.directory.filter(|_| self.parent.is_none()),
            &link,
            self.prices,
            parent,
            on_turn,
        )?;
        increment(&mut self.summary.subagent_sessions)?;
        self.summary.attributed_usage = self
            .summary
            .attributed_usage
            .add(child.as_ref().map_or(TokenUsage::default(), |s| s.usage))?;
        self.summary.attributed_cost = self
            .summary
            .attributed_cost
            .zip(child.as_ref().and_then(|s| s.cost))
            .map(|(a, b)| a + b)
            .filter(|n| n.is_finite());
        let Some(turn) = &mut self.current else {
            return Ok(0);
        };
        let index = turn.subagents.len();
        turn.subagents.push(SubagentStats {
            session_id: link.session_id,
            tool_call_id: link.tool_call_id,
            agent: link.agent,
            relative_path: link.relative_path,
            turns_used: None,
            completed: None,
            session: child,
        });
        Ok(index)
    }
}

fn read_child(
    directory: Option<&Path>,
    link: &ChildLink,
    prices: Option<&PriceTable>,
    parent: ParentTurn,
    on_turn: &mut dyn FnMut(TurnStats),
) -> Result<Option<SessionStats>, Error> {
    let Some(relative) = link.relative_path.as_deref() else {
        return Ok(None);
    };
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::InvalidChildPath);
    }
    let Some(directory) = directory else {
        return Ok(None);
    };
    let child_dir = directory.join(relative);
    let open = |file: &str| match File::open(child_dir.join(file)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::Io(error)),
    };
    let Some(metadata_file) = open("meta.json")? else {
        return Ok(None);
    };
    let Some(messages) = open("messages.jsonl")? else {
        return Ok(None);
    };
    let metadata = parse_metadata(metadata_file)?;
    if metadata.session_id != link.session_id
        || metadata
            .parent_session_id
            .as_ref()
            .is_some_and(|id| id != &parent.session_id)
    {
        return Err(Error::ChildIdentityMismatch);
    }
    Ok(Some(run(
        metadata,
        messages,
        Some(&child_dir),
        prices,
        Some(parent),
        on_turn,
    )?))
}
