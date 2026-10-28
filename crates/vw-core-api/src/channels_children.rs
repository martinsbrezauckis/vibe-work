//! Child callback ownership and scoped interrupt routing. Uses the parent channel; called by `stop_tree` and `pump`.
use super::{Channels, Connection, locked};

pub(crate) struct Control<'a> {
    channels: &'a Channels,
    child: String,
}
impl Drop for Control<'_> {
    fn drop(&mut self) {
        let mut controls = locked(&self.channels.child_controls);
        if let Some((_, count)) = controls.get_mut(&self.child) {
            *count -= 1;
            if *count == 0 {
                controls.remove(&self.child);
            }
        }
    }
}
impl Channels {
    /// Parent carrying a subagent's approval card, for app-wide activity decisions.
    pub(crate) fn callback_parent(&self, callback_id: &str) -> Option<String> {
        let channel_id = *locked(&self.guest_callbacks).get(callback_id)?;
        locked(&self.inner)
            .slots
            .iter()
            .find(|(_, slot)| slot.key.1 == channel_id && slot.alive())
            .map(|(id, _)| id.clone())
    }

    /// Keep verified ownership only for this gated request, also if its future is cancelled.
    pub(crate) fn control_child(&self, child: &str, parent: &str) -> Control<'_> {
        let mut controls = locked(&self.child_controls);
        let (_, count) = controls
            .entry(child.into())
            .or_insert_with(|| (parent.into(), 0));
        *count += 1;
        Control {
            channels: self,
            child: child.into(),
        }
    }

    pub(super) fn child_control(&self, child: &str) -> Option<Connection> {
        let parent = locked(&self.child_controls)
            .get(child)
            .map(|(parent, _)| parent.clone());
        parent.and_then(|parent| self.bound(&parent))
    }
}
