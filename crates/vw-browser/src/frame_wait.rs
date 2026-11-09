//! Text wait conditions across the same verified visible frames used by reads.
//! Depends on the native frame validator; returns only a boolean and never replaces the ref baseline.
use crate::{Browser, scripts};

impl Browser {
    pub(crate) async fn text_in_frames(&self, session: &str, text: &str) -> bool {
        let Ok(page) = self.page(session).await else {
            return false;
        };
        let Some(mount) = self.service.page_mount(session) else {
            return false;
        };
        let script = scripts::frame_text(&page, text);
        let Ok(Some(model)) = self.observe_frames(session, &mount, &page, &script).await else {
            return false;
        };
        self.service.page_mount(session).as_ref() == Some(&mount)
            && self
                .service
                .reading_allowed(session, &page, &model.url)
                .is_ok()
            && !model.text.is_empty()
    }
}
