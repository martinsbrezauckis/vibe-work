//! Archive refusal on a core without Vibe: only a chat Vibe never stored is refused.
use serde_json::{Value, json};
use vw_session::LoadKind;

fn archive(shared: &crate::core::Shared, id: &str) -> Result<(), crate::CoreError> {
    super::check_archive(
        shared,
        "session/archive",
        &json!({"sessionId": id, "archived": true}),
    )
}

fn state(id: &str, bumped: &Value, history: &Value) -> Value {
    json!({"session": {"id": id, "harness": "unified", "createdAt": 1000, "bumpedAt": bumped,
        "title": null, "status": {"type": "idle"}}, "eventId": 0, "turns": [], "history": history})
}

/// A chat rewound in place to its first or a later message has no turns left in Vibe's answer, but Vibe stored it
/// at its first message (`bumpedAt` moved past `createdAt`): it can be archived. A chat that never had a message is
/// still refused.
#[tokio::test]
async fn a_rewound_chat_can_be_archived_and_a_never_started_one_cannot() {
    let core = crate::offline_core::start("archive-rewound").await;
    let shared = &core.shared;
    let user = json!({"id": "m1", "type": "message", "role": "user", "content": []});
    let loads = [
        ("fresh", state("fresh", &json!(1000), &json!([]))),
        // Rewound to the first message: one entry left, no turns.
        (
            "rewound-first",
            state(
                "rewound-first",
                &json!(5000),
                &json!([{"id": "n1", "type": "notice"}]),
            ),
        ),
        // Rewound to the second message: the first turn's entries stay, no turns.
        (
            "rewound-second",
            state(
                "rewound-second",
                &json!(9000),
                &json!([user, {"id": "a1"}, {"id": "r1"}, {"id": "x"}]),
            ),
        ),
    ];
    for (_, loaded) in loads {
        let _ = shared.store.load(loaded, LoadKind::Replace).unwrap();
    }
    let refused = archive(shared, "fresh").expect_err("a chat that never started is refused");
    assert!(
        refused.message.contains("hasn't started yet"),
        "{refused:?}"
    );
    archive(shared, "rewound-first").expect("a rewound chat Vibe stored can be archived");
    archive(shared, "rewound-second").expect("a rewound chat Vibe stored can be archived");
}
