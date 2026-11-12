//! Main desktop-window focus through a replaceable Tauri channel, independent of child webview focus.
//! Called by the typed desktop subscription and native window events; no page may claim native focus.
use std::sync::{Mutex, PoisonError};
use tauri::{AppHandle, Manager, ipc::Channel};

#[derive(Default)]
pub(crate) struct WindowFocus(Mutex<Option<Channel<bool>>>);

pub(crate) fn subscribe(app: &AppHandle, focused: Channel<bool>) -> Result<(), String> {
    let window = crate::main_window(app).ok_or("The main window is unavailable.")?;
    let state = app.state::<WindowFocus>();
    let mut channel = state.0.lock().unwrap_or_else(PoisonError::into_inner);
    let active = window.is_focused().map_err(|error| error.to_string())?;
    focused.send(active).map_err(|error| error.to_string())?;
    *channel = Some(focused);
    Ok(())
}

pub(crate) fn report(window: &tauri::Window, focused: bool) {
    if let Some(state) = window.try_state::<WindowFocus>() {
        let mut channel = state.0.lock().unwrap_or_else(PoisonError::into_inner);
        if channel
            .as_ref()
            .is_some_and(|channel| channel.send(focused).is_err())
        {
            *channel = None;
        }
    }
}
