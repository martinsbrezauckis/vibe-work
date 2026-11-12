//! Keeps Tauri's native file/message commands without replacing web pages' synchronous alert/confirm globals.
//! The app calls the plugin's typed commands; browser webviews use their engine's real page dialogs.
//! Depends on: Tauri native dialog commands.
//! Called by: app plugin registration.
use tauri::{
    AppHandle, RunEvent, Webview, Window, Wry,
    ipc::Invoke,
    plugin::{Plugin, TauriPlugin},
    webview::PageLoadPayload,
};

pub(crate) struct NativeDialogs(TauriPlugin<Wry>);
pub(crate) fn init() -> NativeDialogs {
    NativeDialogs(tauri_plugin_dialog::init())
}
impl Plugin<Wry> for NativeDialogs {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn initialize(
        &mut self,
        app: &AppHandle<Wry>,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }
    // Deliberately no initialization_script: the upstream shim returns an asynchronous Promise for confirm,
    // which a synchronous website reads as true, and tries app IPC from browser pages with no such permission.
    fn window_created(&mut self, window: Window<Wry>) {
        self.0.window_created(window);
    }
    fn webview_created(&mut self, webview: Webview<Wry>) {
        self.0.webview_created(webview);
    }
    fn on_navigation(&mut self, webview: &Webview<Wry>, url: &tauri::Url) -> bool {
        self.0.on_navigation(webview, url)
    }
    fn on_page_load(&mut self, webview: &Webview<Wry>, payload: &PageLoadPayload<'_>) {
        self.0.on_page_load(webview, payload);
    }
    fn on_event(&mut self, app: &AppHandle<Wry>, event: &RunEvent) {
        self.0.on_event(app, event);
    }
    fn extend_api(&mut self, invoke: Invoke<Wry>) -> bool {
        self.0.extend_api(invoke)
    }
}
