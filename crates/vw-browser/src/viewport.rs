//! Responsive page size controls and native readback; device identity and input remain unchanged.
//! Called by the shared action loop after common tabs.emulate permission checks.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};

/// An explicit CSS size or reset to the panel's existing device-width selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Explicit CSS dimensions; `None` restores normal size and color.
    Size(Option<(u32, u32)>),
    /// Native media preference; `None` inherits the system preference.
    Color(Option<bool>),
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        for key in ["mobile", "deviceScaleFactor", "touch", "userAgent"] {
            if args.get(key).is_some() {
                return Err(format!(
                    "The built-in responsive preview does not override {key}; use width/height or a phone/tablet preset."
                ));
            }
        }
        if args.get("reset").is_some_and(|v| !v.is_boolean()) {
            return Err("reset must be true or false; use true to restore the panel size.".into());
        }
        let dimensions = args.get("width").is_some() || args.get("height").is_some();
        let preset = args.get("preset");
        if let Some(scheme) = args.get("colorScheme") {
            if dimensions || preset.is_some() || args["reset"] == true {
                return Err("Change colorScheme separately from size or reset.".into());
            }
            return match scheme.as_str() {
                Some("light") => Ok(Self::Color(Some(false))),
                Some("dark") => Ok(Self::Color(Some(true))),
                Some("system") => Ok(Self::Color(None)),
                _ => Err("colorScheme must be light, dark or system.".into()),
            };
        }
        if args["reset"] == true {
            if dimensions || preset.is_some() {
                return Err("reset clears the size; omit width, height and preset.".into());
            }
            return Ok(Self::Size(None));
        }
        if dimensions && preset.is_some() {
            return Err("Choose a preset or explicit width and height.".into());
        }
        let size = if let Some(preset) = preset {
            match preset.as_str() {
                Some("phone") => (390, 844),
                Some("tablet") => (768, 1024),
                Some("desktop") => (1280, 800),
                _ => return Err("Unknown preset; use phone, tablet or desktop.".into()),
            }
        } else {
            let number = |key: &str| {
                args[key]
                    .as_u64()
                    .filter(|n| (240..=4096).contains(n))
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        format!("{key} must be 240–4096 CSS pixels; supply both width and height.")
                    })
            };
            (number("width")?, number("height")?)
        };
        Ok(Self::Size(Some(size)))
    }
}

impl Browser {
    pub(crate) async fn viewport(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let size = match *request {
            Request::Size(size) => size,
            Request::Color(dark) => return self.color_scheme(session, dark, by).await,
        };
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        if size.is_none() {
            self.service
                .set_color_scheme(session, None)
                .await
                .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        }
        self.service
            .set_viewport(session, size)
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        let mut actual = None;
        for _ in 0..20 {
            actual = self
                .service
                .layout_viewport(session)
                .await
                .map_err(|e| failed(crate::driver::preview_text(&e)))?;
            if actual.is_some_and(|(w, h)| {
                size.is_none_or(|(rw, rh)| {
                    (w - f64::from(rw)).abs() <= 2.0 && (h - f64::from(rh)).abs() <= 2.0
                })
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        bound.check_read_mount().map_err(refused)?;
        let (width, height) = actual.ok_or_else(|| {
            failed(
                "The native viewport could not be read; read the page before trying another size.",
            )
        })?;
        if size.is_some_and(|(w, h)| {
            (width - f64::from(w)).abs() > 2.0 || (height - f64::from(h)).abs() > 2.0
        }) {
            return Err(failed(format!(
                "The native viewport is {width:.0}×{height:.0}, different from the requested size; enlarge the Browser panel or reset the size."
            )));
        }
        Ok(
            json!({"ok":true,"Outcome":"Responsive viewport updated and measured.","action":"tabs.emulate","viewport":{"width":width,"height":height},"reset":size.is_none(),"deviceIdentityChanged":false}),
        )
    }
}
