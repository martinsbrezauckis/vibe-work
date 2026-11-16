//! Installs the host's watch of the person's own pointer presses and releases on a preview webview, at mount
//! (or at the first checked click), so a checked click knows whether the person is clicking in this browser, and a
//! person's press ends the agent act window of that view (`act_window::person_input`, which decides which presses
//! count). The same watch swallows the person's input while the agent controls the view's chat browser
//! (`vw_preview::person_block`, the agent control frame's input block) and reports each press it swallowed
//! (`person_block::refused`: the app window's own focus does not move then, so the window must be told).
//! Depends on: vw-platform `native_click::person`. Called by: `engine` (mount, native input).
use std::sync::Arc;
use tauri::{Webview, Wry};
use vw_preview::{EngineError, PersonPointer};

pub(crate) async fn install(
    webview: &Webview<Wry>,
    label: &str,
) -> Result<Arc<PersonPointer>, EngineError> {
    let person = Arc::new(PersonPointer::default());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let seen = person.clone();
        let label = label.to_owned();
        let blocked: vw_platform::native_click::person::block::Blocked =
            std::sync::Arc::new(vw_preview::person_block::reader(&label));
        let refused: vw_platform::native_click::person::block::Refused =
            std::sync::Arc::new(vw_preview::person_block::refusal(&label));
        crate::platform::with_native(
            webview,
            "watching the person's pointer",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                #[cfg(target_os = "linux")]
                let result = {
                    vw_platform::native_click::person::watch(
                        &native.inner(),
                        move |pressed| {
                            seen.observed();
                            crate::act_window::person_input(&label, pressed);
                        },
                        blocked,
                        refused,
                    );
                    Ok(())
                };
                #[cfg(target_os = "macos")]
                let result = vw_platform::native_click::person::watch(
                    native.inner(),
                    Box::new(move |pressed| {
                        seen.observed();
                        crate::act_window::person_input(&label, pressed);
                    }),
                    blocked,
                    refused,
                );
                let _ = reply.send(result);
            },
        )
        .await?;
    }
    #[cfg(windows)]
    {
        // The hook thread holds only a weak reference: a closed view's watch ends at the next press.
        let seen = Arc::downgrade(&person);
        let label = label.to_owned();
        let blocked: vw_platform::native_click::person::block::Blocked =
            Arc::new(vw_preview::person_block::reader(&label));
        let refused: vw_platform::native_click::person::block::Refused =
            Arc::new(vw_preview::person_block::refusal(&label));
        crate::platform::with_native(
            webview,
            "watching the person's pointer",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                let _ = reply.send(vw_platform::native_click::person::watch(
                    &native.controller(),
                    Box::new(move |pressed| {
                        seen.upgrade()
                            .map(|person| {
                                person.observed();
                                crate::act_window::person_input(&label, pressed);
                            })
                            .is_some()
                    }),
                    blocked,
                    refused,
                ));
            },
        )
        .await?;
    }
    Ok(person)
}
