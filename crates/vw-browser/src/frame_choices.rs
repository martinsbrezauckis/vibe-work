//! Choice inventories from native frame observations, with their host-verified document identity.
//! Called by the frame merger; main-document dialogs and covering children retain the same ref authority.
use super::{FrameRef, Observation};
use crate::model::PageModel;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn append_main(
    model: &mut PageModel,
    observation: &Observation,
    frame: &FrameRef,
    main: bool,
) {
    if !main {
        return;
    }
    for inventory in &observation.model.covering_frames {
        let mut inventory = inventory.clone();
        for item in &mut inventory.choices {
            item.local_bounds = item.bounds;
            item.local_viewport = (observation.width as u32, observation.height as u32);
            item.frame_id = Some(frame.clone());
        }
        model.covering_frames.push(inventory);
    }
}

pub(super) fn append_child(
    model: &mut PageModel,
    observation: &Observation,
    choices: Vec<crate::model::RefItem>,
) {
    if let Some(mut context) = observation.model.document_context.clone() {
        context.frame_origin.clone_from(&observation.origin);
        model.choice_context.push(context);
    }
    model.covering_frames.push(crate::model::FrameChoices {
        origin: observation.origin.clone(),
        complete: !observation.model.truncated
            && choices.len() == observation.model.choice_count as usize,
        total: observation.model.choice_count,
        choices,
    });
}
