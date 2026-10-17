//! Fixture and falsifier tests for the public streaming scanner; depends on synthetic stored files.
//! Checks observed counters, honest unknowns, child attribution, supplied pricing and bounded reads.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use std::cell::Cell;
use std::io::{self, Read};
use std::path::PathBuf;
use std::rc::Rc;
use vw_session_stats::{
    Error, ModelPrices, PriceTable, SessionStats, TokenUsage, TurnStats, scan_readers, scan_session,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn load(name: &str, prices: Option<&PriceTable>) -> (SessionStats, Vec<TurnStats>) {
    let mut turns = Vec::new();
    let summary = scan_session(&fixture(name), prices, |turn| turns.push(turn)).unwrap();
    (summary, turns)
}

fn usage(input: u64, output: u64, cached: u64) -> TokenUsage {
    TokenUsage {
        input: Some(input),
        output: Some(output),
        cached: Some(cached),
    }
}

fn uniform() -> PriceTable {
    PriceTable::uniform(ModelPrices::new(2.0, 4.0, 0.5).unwrap())
}

fn from_values(meta: &Value, messages: &str) -> Result<(SessionStats, Vec<TurnStats>), Error> {
    let metadata = serde_json::to_vec(meta).unwrap();
    let mut turns = Vec::new();
    let stats = scan_readers(metadata.as_slice(), messages.as_bytes(), None, |turn| {
        turns.push(turn);
    })?;
    Ok((stats, turns))
}

#[test]
fn normal_preserves_counters_without_inventing_turn_measurements() {
    let (session, turns) = load("normal", None);
    assert_eq!(session.turns, 1);
    assert_eq!(session.messages, 2);
    assert_eq!(session.usage, usage(1000, 200, 400));
    assert_eq!(session.recorded_steps, Some(1));
    assert_eq!(session.last_model_call.usage, session.usage);
    assert_eq!(session.last_model_call.duration_seconds, Some(2.5));
    assert_eq!(session.model_snapshot.alias.as_deref(), Some("fast"));
    assert_eq!(
        session.model_snapshot.name.as_deref(),
        Some("synthetic-model")
    );
    assert_eq!(session.model_snapshot.provider.as_deref(), Some("mistral"));
    assert_eq!(session.model, None);
    assert_eq!(session.cost, None);
    assert_eq!(session.wall_seconds, None);
    let turn = &turns[0];
    assert_eq!(turn.message_id.as_deref(), Some("normal-user"));
    assert_eq!(turn.usage, TokenUsage::default());
    assert_eq!(turn.model, None);
    assert_eq!(turn.wall_seconds, None);
    assert_eq!(turn.model_seconds, None);
    assert_eq!(turn.succeeded, None);
    assert_eq!(turn.tool_seconds, Some(0.0));
}

#[test]
fn tools_are_counted_by_request_and_duration_not_context_messages() {
    let (session, turns) = load("tools", None);
    assert_eq!(session.turns, 1);
    assert_eq!(session.messages, 7);
    assert_eq!(session.tools.total.calls, 2);
    assert_eq!(session.tools.total.results, 2);
    assert_eq!(session.tools.total.timed_calls, 2);
    assert_eq!(session.tool_seconds, Some(1.75));
    assert_eq!(session.tools.by_kind["bash"].duration_seconds(), Some(1.25));
    assert_eq!(session.tools.by_kind["read_file"].calls, 1);
    assert_eq!(turns[0].tools, session.tools);
    assert_eq!(turns[0].model_seconds, None);
}

#[test]
fn subagent_turns_and_usage_are_attributed_once_to_parent_call() {
    let (session, turns) = load("subagent", Some(&uniform()));
    assert_eq!(turns.len(), 2);
    let child = &turns[0];
    let parent = &turns[1];
    assert_eq!(child.session_id, "child");
    let attribution = child.parent.as_ref().unwrap();
    assert_eq!(attribution.session_id, "parent");
    assert_eq!(attribution.turn_index, Some(1));
    assert_eq!(attribution.tool_call_id, "task-1");
    assert_eq!(parent.session_id, "parent");
    let task = &parent.subagents[0];
    assert_eq!(task.turns_used, Some(2));
    assert_eq!(task.completed, Some(true));
    let child_summary = task.session.as_ref().unwrap();
    assert_eq!(child_summary.turns, 1);
    assert_eq!(child_summary.usage, usage(600, 80, 100));
    assert_eq!(child_summary.tool_seconds, Some(0.25));
    assert_eq!(session.usage, usage(400, 120, 50));
    assert_eq!(session.attributed_usage, usage(1000, 200, 150));
    assert_eq!(session.subagent_sessions, 1);
    assert_eq!(session.tools.total.calls, 1);
    assert_eq!(session.tool_seconds, Some(3.0));
    assert!((session.attributed_cost.unwrap() - 0.002_575).abs() < 1e-12);
}

#[test]
fn failed_turn_never_inherits_stale_usage_or_guesses_success_from_text() {
    let (session, turns) = load("failed", None);
    assert_eq!(session.turns, 2);
    assert_eq!(session.last_model_call.usage, usage(50, 20, 0));
    assert_eq!(turns[1].usage, TokenUsage::default());
    assert_eq!(turns[1].wall_seconds, None);
    assert_eq!(turns[1].model_seconds, None);
    assert_eq!(turns[1].succeeded, None);
    assert_eq!(turns[1].tools.total.results, 1);
    assert_eq!(turns[1].tool_seconds, None);
    assert_eq!(session.tool_seconds, None);
}

#[test]
fn missing_usage_stays_unknown_including_when_prices_are_supplied() {
    let (session, turns) = load("without-usage", Some(&uniform()));
    assert_eq!(session.turns, 1);
    assert_eq!(session.usage, TokenUsage::default());
    assert_eq!(session.last_model_call.duration_seconds, None);
    assert_eq!(session.cost, None);
    assert_eq!(turns[0].cost, None);
    let serialized = serde_json::to_value(&turns[0]).unwrap();
    assert!(serialized["usage"]["input"].is_null());
    assert!(serialized["wall_seconds"].is_null());
}

#[test]
fn caller_uniform_prices_charge_cached_input_once_and_ignore_stored_prices() {
    let (session, _) = load("normal", Some(&uniform()));
    assert!((session.cost.unwrap() - 0.0022).abs() < 1e-12);
    assert_eq!(session.attributed_cost, session.cost);
    let mut models = PriceTable::default();
    models.insert("synthetic-model", ModelPrices::new(2.0, 4.0, 0.5).unwrap());
    assert_eq!(load("normal", Some(&models)).0.cost, None);
    assert!(
        (models
            .cost(Some("synthetic-model"), usage(1000, 200, 400))
            .unwrap()
            - 0.0022)
            .abs()
            < 1e-12
    );
    assert_eq!(
        models.cost(Some("other-model"), usage(1000, 200, 400)),
        None
    );
}

#[test]
fn pricing_missing_cache_is_unknown_unless_input_rates_are_equal() {
    let no_cache = TokenUsage {
        input: Some(1000),
        output: Some(200),
        cached: None,
    };
    assert_eq!(uniform().cost(None, no_cache), None);
    let same_rate = PriceTable::uniform(ModelPrices::new(2.0, 4.0, 2.0).unwrap());
    assert!((same_rate.cost(None, no_cache).unwrap() - 0.0028).abs() < 1e-12);
    assert!(ModelPrices::new(-1.0, 1.0, 1.0).is_err());
    assert!(ModelPrices::new(f64::NAN, 1.0, 1.0).is_err());
    assert!(ModelPrices::new(f64::INFINITY, 1.0, 1.0).is_err());
    assert_eq!(uniform().cost(None, usage(1, 2, 3)), None);
}

#[test]
fn readers_keep_recorded_task_count_when_child_files_are_unavailable() {
    let meta = std::fs::read(fixture("subagent").join("meta.json")).unwrap();
    let messages = std::fs::read(fixture("subagent").join("messages.jsonl")).unwrap();
    let mut turns = Vec::new();
    let session = scan_readers(meta.as_slice(), messages.as_slice(), None, |turn| {
        turns.push(turn);
    })
    .unwrap();
    assert_eq!(session.subagent_sessions, 1);
    let task = &turns[0].subagents[0];
    assert_eq!(task.turns_used, Some(2));
    assert_eq!(task.completed, Some(true));
    assert!(task.session.is_none());
    assert_eq!(session.attributed_usage, TokenUsage::default());
}

#[test]
fn zero_and_missing_counters_remain_distinct_and_cache_cannot_exceed_input() {
    let (zero, _) = from_values(&json!({"session_id":"zero","stats":{"session_prompt_tokens":0,"session_completion_tokens":0}}), "").unwrap();
    assert_eq!(
        zero.usage,
        TokenUsage {
            input: Some(0),
            output: Some(0),
            cached: None
        }
    );
    assert!(matches!(
        from_values(
            &json!({"session_id":"bad","stats":{"session_prompt_tokens":1,"session_cached_tokens":2}}),
            ""
        ),
        Err(Error::InvalidUsage)
    ));
    assert!(matches!(
        from_values(
            &json!({"session_id":"bad","stats":{"last_turn_duration":-1}}),
            ""
        ),
        Err(Error::InvalidDuration)
    ));
}

#[test]
fn unmatched_duplicate_and_missing_results_never_become_timed_calls() {
    let messages = concat!(
        "{\"role\":\"user\"}\n",
        "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"a\",\"function\":{\"name\":\"bash\"}},{\"id\":\"b\"}]}\n",
        "{\"role\":\"tool\",\"tool_call_id\":\"a\",\"tool_result\":{\"duration\":0.5,\"cancelled\":true}}\n",
        "{\"role\":\"tool\",\"tool_call_id\":\"a\",\"tool_result\":{\"duration\":99}}\n"
    );
    let (session, turns) = from_values(&json!({"session_id":"tools"}), messages).unwrap();
    assert_eq!(session.tools.total.calls, 2);
    assert_eq!(session.tools.unnamed.calls, 1);
    assert_eq!(session.tools.total.cancelled, 1);
    assert_eq!(session.tools.total.recorded_seconds, 0.5);
    assert_eq!(session.tools.unmatched_results, 1);
    assert_eq!(turns[0].tool_seconds, None);
}

#[test]
fn invalid_tool_duration_and_ambiguous_ids_are_errors() {
    let prefix = "{\"role\":\"user\"}\n{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"a\"}]}\n";
    let negative = format!(
        "{prefix}{{\"role\":\"tool\",\"tool_call_id\":\"a\",\"tool_result\":{{\"duration\":-1}}}}\n"
    );
    assert!(matches!(
        from_values(&json!({"session_id":"bad"}), &negative),
        Err(Error::InvalidDuration)
    ));
    let duplicate = "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"a\"},{\"id\":\"a\"}]}\n";
    assert!(matches!(
        from_values(&json!({"session_id":"bad"}), duplicate),
        Err(Error::DuplicateToolCallId)
    ));
}

#[test]
fn malformed_tail_and_io_errors_fail_instead_of_returning_partial_totals() {
    struct BrokenReader;
    impl Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("synthetic read failure"))
        }
    }
    assert!(matches!(
        from_values(&json!({"session_id":"bad"}), "{\"role\":\"user\"}\n{"),
        Err(Error::Json(_))
    ));
    assert!(
        scan_readers(
            b"{\"session_id\":\"bad\"}".as_slice(),
            BrokenReader,
            None,
            |_| {}
        )
        .is_err()
    );
}

#[test]
fn compacted_prefix_calls_are_not_attributed_to_next_user_turn() {
    let messages = "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"prefix\"}]}\n{\"role\":\"user\"}\n{\"role\":\"assistant\"}\n";
    let (session, turns) = from_values(
        &json!({"session_id":"prefix","stats":{"session_prompt_tokens":999}}),
        messages,
    )
    .unwrap();
    assert_eq!(session.unattributed_tool_calls, 1);
    assert_eq!(session.usage.input, Some(999));
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].tools.total.calls, 0);
}

#[test]
fn large_content_is_skipped_without_a_line_or_transcript_buffer() {
    // Lazy repeated segments avoid allocating either a large string or a full transcript.
    let opening = b"{\"role\":\"user\",\"content\":\"".as_slice();
    let closing = b"\"}\n{\"role\":\"assistant\"}\n{\"role\":\"user\"}\n".as_slice();
    let huge = io::repeat(b'x').take(8 * 1024 * 1024);
    let mut count = 0;
    let session = scan_readers(
        b"{\"session_id\":\"large\"}".as_slice(),
        opening.chain(huge).chain(closing),
        None,
        |_| count += 1,
    )
    .unwrap();
    assert_eq!(count, 2);
    assert_eq!(session.messages, 3);
}

#[test]
fn recorded_tool_outcomes_are_preserved_without_assigning_them_to_turns() {
    let (session, _) = load("failed", None);
    let value = serde_json::to_value(session).unwrap();
    assert_eq!(value["recorded_tools"]["failed"], json!(1));
    assert!(value["recorded_tools"]["succeeded"].is_null());
    assert!(value["recorded_tools"]["rejected"].is_null());
}

#[test]
fn callbacks_run_before_the_remaining_transcript_is_read() {
    struct GatedReader {
        head: io::Cursor<&'static [u8]>,
        tail: io::Cursor<&'static [u8]>,
        emitted: Rc<Cell<bool>>,
    }
    impl Read for GatedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let read = self.head.read(buffer)?;
            if read != 0 {
                return Ok(read);
            }
            if !self.emitted.get() {
                return Err(io::Error::other("callback did not stream"));
            }
            self.tail.read(buffer)
        }
    }
    let emitted = Rc::new(Cell::new(false));
    let reader = GatedReader {
        head: io::Cursor::new(
            b"{\"role\":\"user\"}\n{\"role\":\"assistant\"}\n{\"role\":\"user\"}\n",
        ),
        tail: io::Cursor::new(b"{\"role\":\"assistant\"}\n"),
        emitted: emitted.clone(),
    };
    let session = scan_readers(
        b"{\"session_id\":\"stream\"}".as_slice(),
        reader,
        None,
        |_| emitted.set(true),
    )
    .unwrap();
    assert_eq!(session.turns, 2);
}

#[test]
fn child_links_missing_paths_and_files_leave_aggregate_usage_unknown() {
    let messages = "{\"role\":\"user\"}\n{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"task\",\"function\":{\"name\":\"task\"}}]}\n";
    let meta = json!({"session_id":"parent","child_sessions":[{"session_id":"child","tool_call_id":"task","agent":"explore"}],"stats":{"session_prompt_tokens":1,"session_completion_tokens":1,"session_cached_tokens":0}});
    let (session, turns) = from_values(&meta, messages).unwrap();
    assert_eq!(session.usage, usage(1, 1, 0));
    assert_eq!(session.attributed_usage, TokenUsage::default());
    assert!(turns[0].subagents[0].session.is_none());
    assert_eq!(turns[0].subagents[0].turns_used, None);
    let mut bad_path = meta.clone();
    bad_path["child_sessions"][0]["relative_path"] = json!("../outside");
    assert!(matches!(
        from_values(&bad_path, messages),
        Err(Error::InvalidChildPath)
    ));
    let mut duplicate = meta;
    let link = duplicate["child_sessions"][0].clone();
    duplicate["child_sessions"]
        .as_array_mut()
        .unwrap()
        .push(link);
    assert!(matches!(
        from_values(&duplicate, messages),
        Err(Error::DuplicateChildLink)
    ));
}

#[test]
fn duplicate_task_results_and_reused_call_ids_do_not_double_count_child_usage() {
    let meta = std::fs::read(fixture("subagent").join("meta.json")).unwrap();
    let mut messages = std::fs::read_to_string(fixture("subagent").join("messages.jsonl")).unwrap();
    messages.push_str("{\"role\":\"tool\",\"tool_call_id\":\"task-1\",\"tool_result\":{\"duration\":99,\"output\":{\"turns_used\":999}}}\n{\"role\":\"user\"}\n{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"task-1\",\"function\":{\"name\":\"task\"}}]}\n");
    let mut turns = Vec::new();
    let session = scan_readers(meta.as_slice(), messages.as_bytes(), None, |turn| {
        turns.push(turn);
    })
    .unwrap();
    assert_eq!(session.subagent_sessions, 1);
    assert_eq!(session.tools.total.calls, 2);
    assert_eq!(session.tools.total.recorded_seconds, 3.0);
    assert_eq!(session.tools.unmatched_results, 1);
    assert_eq!(turns[0].subagents[0].turns_used, Some(2));
    assert_eq!(turns[1].subagents, []);
}

#[test]
fn ordinary_tool_output_can_use_task_field_names_with_other_types() {
    let messages = "{\"role\":\"user\"}\n{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"mcp\",\"function\":{\"name\":\"plugin_tool\"}}]}\n{\"role\":\"tool\",\"tool_call_id\":\"mcp\",\"tool_result\":{\"duration\":1,\"output\":{\"completed\":\"yes\",\"turns_used\":{\"text\":\"not task usage\"}}}}\n";
    let (session, turns) = from_values(&json!({"session_id":"plugin"}), messages).unwrap();
    assert_eq!(session.tools.total.calls, 1);
    assert_eq!(session.tool_seconds, Some(1.0));
    assert_eq!(turns[0].subagents, []);
}

fn staged_subagent() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().unwrap();
    let source = fixture("subagent");
    for file in [
        "meta.json",
        "messages.jsonl",
        "subagents/child/meta.json",
        "subagents/child/messages.jsonl",
    ] {
        let target = temporary.path().join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(source.join(file), target).unwrap();
    }
    temporary
}

fn edit_meta(path: &std::path::Path, edit: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn absent_child_file_is_unknown_and_disagreeing_child_identity_is_an_error() {
    let temporary = staged_subagent();
    let child_meta = temporary.path().join("subagents/child/meta.json");
    edit_meta(&child_meta, |value| {
        value["parent_session_id"] = json!("different-parent");
    });
    assert!(matches!(
        scan_session(temporary.path(), None, |_| {}),
        Err(Error::ChildIdentityMismatch)
    ));
    edit_meta(&child_meta, |value| {
        value["parent_session_id"] = json!("parent");
        value["session_id"] = json!("different-child");
    });
    assert!(matches!(
        scan_session(temporary.path(), None, |_| {}),
        Err(Error::ChildIdentityMismatch)
    ));
    std::fs::remove_file(child_meta).unwrap();
    let mut turns = Vec::new();
    let session =
        scan_session(temporary.path(), Some(&uniform()), |turn| turns.push(turn)).unwrap();
    assert_eq!(turns.len(), 1);
    assert!(turns[0].subagents[0].session.is_none());
    assert_eq!(turns[0].subagents[0].turns_used, Some(2));
    assert_eq!(session.attributed_usage, TokenUsage::default());
    assert_eq!(session.attributed_cost, None);
}

#[test]
fn child_usage_missing_one_counter_preserves_the_other_aggregate_counters() {
    let temporary = staged_subagent();
    edit_meta(
        &temporary.path().join("subagents/child/meta.json"),
        |value| {
            value["stats"]
                .as_object_mut()
                .unwrap()
                .remove("session_cached_tokens");
        },
    );
    let session = scan_session(temporary.path(), Some(&uniform()), |_| {}).unwrap();
    assert_eq!(
        session.attributed_usage,
        TokenUsage {
            input: Some(1000),
            output: Some(200),
            cached: None
        }
    );
    assert_eq!(session.attributed_cost, None);
}

#[test]
fn child_usage_overflow_fails_instead_of_wrapping_or_saturating() {
    let temporary = staged_subagent();
    edit_meta(&temporary.path().join("meta.json"), |value| {
        value["stats"]["session_prompt_tokens"] = json!(u64::MAX);
    });
    assert!(matches!(
        scan_session(temporary.path(), None, |_| {}),
        Err(Error::Overflow)
    ));
}

#[test]
fn negative_or_fractional_task_counts_are_unknown_and_interruption_is_preserved() {
    let meta: Value =
        serde_json::from_slice(&std::fs::read(fixture("subagent").join("meta.json")).unwrap())
            .unwrap();
    for count in [json!(-1), json!(1.5), json!([1])] {
        let messages = format!(
            "{{\"role\":\"user\"}}\n{{\"role\":\"assistant\",\"tool_calls\":[{{\"id\":\"task-1\",\"function\":{{\"name\":\"task\"}}}}]}}\n{{\"role\":\"tool\",\"tool_call_id\":\"task-1\",\"tool_result\":{{\"output\":{{\"turns_used\":{count},\"completed\":false}}}}}}\n"
        );
        let (_, turns) = from_values(&meta, &messages).unwrap();
        assert_eq!(turns[0].subagents[0].turns_used, None);
        assert_eq!(turns[0].subagents[0].completed, Some(false));
    }
}
