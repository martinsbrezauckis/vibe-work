//! Native light/dark preview and isolated-world media-query readback, called by viewport dispatch.
//! Depends on the mounted-page permission fence; no page CSS or JavaScript API is replaced.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};

impl Browser {
    pub(crate) async fn color_scheme(
        &self,
        session: &str,
        dark: Option<bool>,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        self.service
            .set_color_scheme(session, dark)
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        let mut actual = None;
        for _ in 0..20 {
            let value = self
                .service
                .evaluate_on_mount_checked(
                    mount,
                    "JSON.stringify({dark:matchMedia('(prefers-color-scheme: dark)').matches})"
                        .into(),
                    self.dispatch_permission_check(session, by),
                )
                .await
                .map_err(|e| failed(crate::driver::preview_text(&e)))?;
            let value: Value = crate::model::parse_answer(&value).map_err(failed)?;
            actual = value["dark"].as_bool();
            if actual.is_some() && dark.is_none_or(|d| actual == Some(d)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        bound.check_read_mount().map_err(refused)?;
        let actual = actual.ok_or_else(|| {
            failed("The page did not return its color preference; read the page before retrying.")
        })?;
        if dark.is_some_and(|d| d != actual) {
            return Err(failed(
                "The page's native color preference did not change; reset colorScheme to system before retrying.",
            ));
        }
        Ok(
            json!({"ok":true,"action":"tabs.emulate","Outcome":"Native color preference updated and read back.","colorScheme":if actual {"dark"} else {"light"},"system":dark.is_none()}),
        )
    }
}
