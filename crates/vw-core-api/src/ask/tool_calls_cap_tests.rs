//! The tool-call record cap: cards that closed without a use (cancelled by a Stop or a turn that ended, refused,
//! cancelled) give their room back, an open card never does.
use std::collections::VecDeque;

use serde_json::json;
use vw_ask::{Choice, Outcome};

use super::{Call, make_room};

fn call(n: usize) -> Call {
    Call {
        session: "s".into(),
        entry: json!({"id": format!("vw-tool-{n}")}),
    }
}

/// 500 tool cards were opened and then cancelled when their turns were stopped; the stopped agents never retried.
/// A new tool approval still gets a card (the oldest closed record makes room), while 500 open cards still refuse.
#[tokio::test]
async fn closed_cards_give_their_room_back_and_open_ones_never_do() {
    let core = crate::offline_core::start("tool-calls-cap").await;
    let shared = &core.shared;
    let mut records: VecDeque<Call> = (0..500).map(call).collect();
    for n in 0..500 {
        let _ = shared.asks.open_callback(
            "approval",
            Some("s".into()),
            vec![Choice::Once, Choice::No],
            Some(format!("callback:vw-tool-{n}")),
            false,
        );
    }
    let open = make_room(shared, &mut records);
    assert!(open.is_err(), "500 open cards keep their room");
    for n in 0..500 {
        shared.asks.settle(
            &format!("callback:vw-tool-{n}"),
            Outcome::Cancelled {
                reason: "the turn was stopped".into(),
            },
        );
    }
    let closed = make_room(shared, &mut records);
    assert!(
        closed.is_ok(),
        "closed cards give their room back: {closed:?}"
    );
    assert_eq!(records.len(), 500 - 1);
    assert!(
        records.iter().all(|c| c.entry["id"] != "vw-tool-0"),
        "the oldest closed record went first"
    );
}
