//! Host binding for subtree reads; read.rs supplies limits and uses the ordinary verified frame merger.
//! Retained node identity and document guards prevent a replacement or navigation from reusing the root.
use crate::{Browser, act::Action, scripts};
use serde_json::json;

impl Browser {
    pub(crate) fn subtree_script(
        &self,
        session: &str,
        page: &str,
        reference: &str,
        limits: (usize, usize, usize),
    ) -> Result<String, String> {
        self.check_ref(
            session,
            &Action::Hover {
                reference: reference.into(),
            },
        )
        .map_err(|e| e.text)?;
        let seen = self
            .current_seen(session)
            .ok_or("The page closed; read it again.")?;
        let item = seen
            .items
            .iter()
            .find(|item| item.id == reference)
            .ok_or("The subtree ref is no longer retained; read the page again.")?;
        if item.coordinate_kind.is_some() {
            return Err("This ref is a visual region, not a DOM subtree; use a selector read to obtain a DOM ref.".into());
        }
        let document = item
            .frame_id
            .as_ref()
            .map_or(seen.model.document.as_str(), |frame| {
                frame.document.as_str()
            });
        Ok(scripts::observe_subtree(
            page,
            limits,
            &json!({"ref":reference,"node":item.node,"document":document}),
        ))
    }
}
