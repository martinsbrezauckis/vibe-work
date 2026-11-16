//! The block's rule: it holds exactly while the agent control frame shows Working.
//!
//! PREDICT (written before the module): there is no block at all, so the person's input reaches the built-in
//! browser while the agent works; after the module, the block holds while a turn runs and the agent acted within
//! the activity window, and Pause, the strip's Stop and the turn's end lift it.
use super::{ACTIVE_FOR, Facts, acted, lease};
use std::time::Duration;

fn working() -> Facts {
    Facts {
        turn_running: true,
        in_flight: true,
        ..Facts::default()
    }
}

#[test]
fn the_block_holds_while_the_agent_works_in_a_running_turn() {
    assert_eq!(lease(working()), Some(ACTIVE_FOR));
    let after = Facts {
        in_flight: false,
        since_agent: Some(Duration::from_secs(10)),
        ..working()
    };
    assert_eq!(
        lease(after),
        ACTIVE_FOR.checked_sub(Duration::from_secs(10))
    );
}

#[test]
fn pause_or_strip_stop_and_turn_end_lift_the_block() {
    for facts in [
        Facts {
            person_has_it: true,
            ..working()
        },
        Facts {
            turn_running: false,
            ..working()
        },
    ] {
        assert_eq!(lease(facts), None, "{facts:?}");
    }
}

#[test]
fn the_block_ends_with_the_agents_activity_window() {
    for since in [ACTIVE_FOR, ACTIVE_FOR + Duration::from_secs(1)] {
        let idle = Facts {
            in_flight: false,
            since_agent: Some(since),
            ..working()
        };
        assert_eq!(lease(idle), None);
    }
    let never = Facts {
        in_flight: false,
        ..working()
    };
    assert_eq!(
        lease(never),
        None,
        "a turn without browser actions never blocks"
    );
}

/// PREDICT (written before the change): the block ends 30 s after the agent's last browser action even while its
/// turn still runs (a shell step between two browser steps), while the frame keeps saying Working: the person's keys
/// then land in the agent's field. After the change the block holds for the rest of a turn in which the agent used
/// the browser, until Pause, the strip's Stop or the turn's end.
#[test]
fn the_block_holds_for_the_rest_of_a_turn_in_which_the_agent_used_the_browser() {
    let between_steps = Facts {
        in_flight: false,
        since_agent: Some(ACTIVE_FOR + Duration::from_secs(15)),
        acted_this_turn: true,
        ..working()
    };
    assert_eq!(lease(between_steps), Some(ACTIVE_FOR));
    for lifted in [
        Facts {
            person_has_it: true,
            ..between_steps
        },
        Facts {
            turn_running: false,
            ..between_steps
        },
    ] {
        assert_eq!(lease(lifted), None, "{lifted:?}");
    }
}

#[test]
fn only_the_running_turn_counts_as_the_agents() {
    assert!(acted(Some("t2"), Some("t2")));
    assert!(
        !acted(Some("t1"), Some("t2")),
        "an earlier turn's browser use"
    );
    assert!(!acted(None, Some("t2")));
    assert!(!acted(Some("t2"), None), "no turn runs");
}
