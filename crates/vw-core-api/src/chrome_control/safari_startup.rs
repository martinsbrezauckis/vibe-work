//! Startup repair of the running app's Safari registration, gated by its saved switch and build feature.
//! Core startup calls this once; the existing registration runner works in the background without a notice.
use crate::core::Shared;
use std::{future::Future, path::PathBuf, sync::Arc};
use tokio::task::JoinHandle;

#[cfg(all(feature = "your-safari", target_os = "macos"))]
pub(super) fn start(shared: &Arc<Shared>) {
    drop(start_with(
        shared,
        std::env::current_exe(),
        |app, remove| async move {
            super::safari_registration::change_at(
                std::path::Path::new("/usr/bin/pluginkit"),
                &app,
                remove,
            )
            .await
        },
    ));
}

fn start_with<F, R>(
    shared: &Arc<Shared>,
    executable: std::io::Result<PathBuf>,
    register: R,
) -> Option<JoinHandle<()>>
where
    F: Future<Output = Result<(), String>> + Send + 'static,
    R: FnOnce(PathBuf, bool) -> F + Send + 'static,
{
    if !super::safari::enabled(shared) {
        return None;
    }
    let shared = shared.clone();
    Some(tokio::spawn(async move {
        let _held = shared.chrome_control.lifecycle.lock().await;
        // A switch-off or removal that won the setup lock cancels this startup repair.
        if !super::safari::enabled(&shared) {
            return;
        }
        let app = executable
            .map_err(|reason| reason.to_string())
            .and_then(|exe| {
                super::safari_package::app_at(&exe)
                    .ok_or_else(|| "The running executable is outside an app bundle.".to_owned())
            });
        let result = match app {
            Ok(app) => register(app, false).await,
            Err(reason) => Err(reason),
        };
        if let Err(reason) = result {
            tracing::warn!(%reason, "Safari extension startup registration failed");
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    async fn saved_switch(name: &str, on: bool) -> crate::Core {
        crate::offline_core::start_with(name, |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
            config.chrome_control.user_home = Some(dir.join("browser-home"));
            let state = super::super::owner::state_dir(&config.chrome_control).unwrap();
            std::fs::create_dir_all(&state).unwrap();
            std::fs::write(state.join("safari-enabled.json"), on.to_string()).unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn startup_with_safari_on_registers_the_running_bundle_once() {
        let core = saved_switch("safari-startup-on", true).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let app = PathBuf::from("/Volumes/Moved copy/Vibe Work.app");
        let task = start_with(
            &core.shared,
            Ok(app.join("Contents/MacOS/vibe-work")),
            move |app, remove| async move {
                recorded.lock().unwrap().push((app, remove));
                Ok(())
            },
        );
        if let Some(task) = task {
            task.await.unwrap();
        }
        let expected = if cfg!(feature = "your-safari") {
            vec![(app, false)]
        } else {
            Vec::new()
        };
        assert_eq!(*calls.lock().unwrap(), expected);
        core.shutdown().await;
    }

    #[tokio::test]
    async fn startup_with_safari_off_never_registers() {
        let core = saved_switch("safari-startup-off", false).await;
        let task = start_with(
            &core.shared,
            Ok(PathBuf::from(
                "/Applications/Vibe Work.app/Contents/MacOS/vibe-work",
            )),
            |_, _| async { panic!("Safari is switched off") },
        );
        assert!(task.is_none());
        core.shutdown().await;
    }

    #[cfg(feature = "your-safari")]
    #[tokio::test]
    async fn startup_outside_a_bundle_never_registers_another_copy() {
        let core = saved_switch("safari-startup-unbundled", true).await;
        for executable in [
            Ok(PathBuf::from("/tmp/vibe-work")),
            Err(std::io::Error::other("executable unavailable")),
        ] {
            start_with(&core.shared, executable, |_, _| async {
                panic!("startup must not search for another bundle")
            })
            .unwrap()
            .await
            .unwrap();
        }
        core.shutdown().await;
    }

    #[cfg(feature = "your-safari")]
    #[tokio::test]
    async fn startup_registration_failure_finishes_in_the_background_without_a_notice() {
        let core = saved_switch("safari-startup-failure", true).await;
        let mut events = core.shared.events.subscribe();
        let before = super::super::safari::view(&core.shared);
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let task = start_with(
            &core.shared,
            Ok(PathBuf::from(
                "/Applications/Vibe Work.app/Contents/MacOS/vibe-work",
            )),
            |_, remove| async move {
                assert!(!remove);
                started.send(()).unwrap();
                released.await.unwrap();
                Err("registration failed".into())
            },
        )
        .unwrap();
        waiting.await.unwrap();
        assert!(!task.is_finished());
        release.send(()).unwrap();
        task.await.unwrap();
        assert!(super::super::safari::enabled(&core.shared));
        assert_eq!(super::super::safari::view(&core.shared), before);
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                crate::CoreEvent::Change {
                    change: crate::event::SessionChange::Notice { .. }
                }
            ));
        }
        core.shutdown().await;
    }
}
