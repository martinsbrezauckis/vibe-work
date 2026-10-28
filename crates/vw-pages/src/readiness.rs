//! Completes rendered handbook loads when native engines omit a finish event (including fragment navigation).
//! Called by the viewer's drain; reads the current document and checks the exact mounted target before updating.

use crate::service::{PagesService, bounded, mount_id};

impl PagesService {
    pub(crate) async fn refresh_document(&self, label: &str) {
        let target = {
            let state = self.shared.lock();
            state
                .mount
                .as_ref()
                .filter(|m| m.label == label)
                .and_then(|m| {
                    let viewer = state.viewers.get(&m.session)?;
                    (viewer.loading || viewer.title.is_none())
                        .then(|| viewer.url().map(|url| (m.clone(), url)))?
                })
        };
        let Some((mount, expected)) = target else {
            return;
        };
        let Ok(raw) = bounded(
            self.config.engine_timeout,
            "handbook document readiness",
            self.engine.evaluate(
                &mount_id(&mount),
                "[String(location.href),String(document.title).slice(0,200),String(document.readyState)]"
                    .into(),
            ),
        )
        .await else {
            return;
        };
        let Ok((url, title, ready)) = serde_json::from_str::<(String, String, String)>(&raw) else {
            return;
        };
        if url != expected.as_str() || ready != "complete" {
            return;
        }
        let mut state = self.shared.lock();
        if state
            .mount
            .as_ref()
            .is_none_or(|m| m.label != label || m.session != mount.session || m.path != mount.path)
        {
            return;
        }
        let Some(viewer) = state.viewers.get_mut(&mount.session) else {
            return;
        };
        if viewer.url().as_ref() != Some(&expected) {
            return;
        }
        viewer.loading = false;
        viewer.title = Some(title.chars().take(200).collect());
        viewer.loaded_mtime = viewer
            .file()
            .and_then(|f| std::fs::metadata(f).ok())
            .and_then(|m| m.modified().ok());
        drop(state);
        self.shared.loads.send_modify(|n| *n += 1);
        self.shared.changed(Some(&mount.session));
    }
}
