//! Compact inventories of covering-frame choices, kept separate from query and main-ref quotas.
//! Used by read serialization and inline fitting; completeness always describes the returned inventory.

use crate::model::PageModel;
use serde_json::{Value, json};

pub(crate) fn include(content: &mut Value, model: &PageModel) {
    if model.covering_frames.is_empty() {
        return;
    }
    content["coveringFrames"] = json!(
        model
            .covering_frames
            .iter()
            .map(|frame| {
                let choices: Vec<Value> = frame
                    .choices
                    .iter()
                    .map(|r| {
                        let mut choice = json!({"ref":r.id,"role":r.role,"name":r.name});
                        if r.disabled == Some(true) {
                            choice["disabled"] = json!(true);
                        }
                        if let Some(expanded) = r.expanded {
                            choice["expanded"] = json!(expanded);
                        }
                        if let Some(checked) = r.checked {
                            choice["checked"] = json!(checked);
                        }
                        if let Some(pressed) = r.pressed {
                            choice["pressed"] = json!(pressed);
                        }
                        choice
                    })
                    .collect();
                json!({"frameOrigin":frame.origin,"coversPage":true,"choicesTotal":frame.total,
            "complete":frame.complete,"choices":choices})
            })
            .collect::<Vec<_>>()
    );
}

/// Large inventories cannot crowd every other field out. Small consent walls fit intact;
/// larger lists retain exact names and explicitly lose their completeness claim.
pub(crate) fn bound(content: &mut Value, budget: usize) -> bool {
    let Some(frames) = content.get_mut("coveringFrames") else {
        return false;
    };
    if crate::observe::size(frames) <= budget {
        return false;
    }
    let Some(frames) = frames.as_array_mut() else {
        return false;
    };
    for frame in frames.iter_mut().rev() {
        if let Some(choices) = frame["choices"].as_array_mut()
            && !choices.is_empty()
        {
            choices.pop();
            frame["complete"] = json!(false);
            return true;
        }
    }
    if frames.len() > 1 {
        frames.pop();
        content["coveringFramesOmitted"] =
            json!(content["coveringFramesOmitted"].as_u64().unwrap_or(0) + 1);
        return true;
    }
    false
}

/// Once an inventory is partial, its tail yields before the ordinary main-document reservation.
pub(crate) fn trim_incomplete(content: &mut Value) -> bool {
    let Some(frames) = content
        .get_mut("coveringFrames")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    frames.iter_mut().rev().any(|frame| {
        frame["complete"] == false
            && frame["choices"]
                .as_array_mut()
                .is_some_and(|choices| choices.pop().is_some())
    })
}

pub(crate) fn unique(model: &PageModel) -> bool {
    let mut unique = std::collections::HashMap::new();
    model.observed_refs().all(|r| {
        let identity = (r.frame_id.as_ref().map(|f| &f.id), r.node);
        unique
            .insert(&r.id, identity)
            .is_none_or(|prior| prior == identity)
    })
}
