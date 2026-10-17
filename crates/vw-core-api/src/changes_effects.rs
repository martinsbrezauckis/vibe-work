//! Non-git Changes groups completed file effects by their owning session and turn.
//! Depends on session facts and Changes wire types; called by `changes::build_view`.
//! Child turns have labels and source ids, but no parent rewind anchor.

use std::path::Path;

use crate::changes::{Hub, Tracker};
use crate::changes_facts::{Facts, shown};
use crate::changes_view::{ChangesMode, ChangesSubagent, ChangesView, TurnChanges, TurnFile};

fn turns_of(
    session: &Facts,
    base: Option<&Path>,
    subagent: Option<&ChangesSubagent>,
) -> Vec<TurnChanges> {
    let mut turns: Vec<TurnChanges> = Vec::new();
    for touch in &session.touches {
        let Some(turn) = touch.turn_id.as_deref().and_then(|id| session.turn(id)) else {
            continue;
        };
        let index = if let Some(index) = turns.iter().position(|known| known.turn_id == turn.id) {
            index
        } else {
            turns.push(TurnChanges {
                turn_id: turn.id.clone(),
                number: turn.number,
                anchor_entry_id: if subagent.is_none() {
                    turn.anchor.clone()
                } else {
                    None
                },
                files: Vec::new(),
                subagent: subagent.cloned(),
            });
            turns.len() - 1
        };
        let current = &mut turns[index];
        let path = shown(&touch.path, base);
        match current.files.iter_mut().find(|file| file.path == path) {
            Some(file) => file.entry_ids.push(touch.entry_id.clone()),
            None => current.files.push(TurnFile {
                path,
                entry_ids: vec![touch.entry_id.clone()],
                created: touch.created,
            }),
        }
    }
    turns.sort_by_key(|turn| turn.number);
    turns
}

/// Files from the session and its descendants, with paths relative to the shown session's folder.
pub(crate) fn effects_view(hub: &Hub, tracker: &Tracker, session: &Facts, view: &mut ChangesView) {
    let base = session.cwd.as_deref();
    let mut turns = turns_of(session, base, None);
    for (child, facts) in &session.subagents {
        turns.extend(turns_of(facts, base, Some(child)));
    }
    if turns.is_empty() {
        view.mode = ChangesMode::None;
        view.fingerprint = "none".into();
        return;
    }
    view.mode = ChangesMode::Effects;
    let changed: Vec<String> = turns
        .iter()
        .flat_map(|turn| turn.files.iter().map(|file| file.path.clone()))
        .collect();
    let dismissed = hub
        .dir
        .as_deref()
        .is_some_and(|dir| vw_changes::offer_dismissed(dir, &tracker.folder));
    view.offer_history = !dismissed && vw_changes::looks_like_code(&tracker.folder, &changed);
    let encoded = serde_json::to_vec(&turns).unwrap_or_default();
    view.fingerprint = format!(
        "effects:{}:{}",
        vw_changes::fnv_hex(&[&encoded]),
        u8::from(view.offer_history)
    );
    view.turns = turns;
}
