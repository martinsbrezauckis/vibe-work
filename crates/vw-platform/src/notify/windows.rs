//! `WinRT` toast creation and subscription ownership for `DesktopNotifier`, including short-lived callers.
//! Depends on windows and the portable store; OS callback threads take entries before invoking a click.
//! Expiry removes the toast: this unpackaged app has no COM activator for Notification Center clicks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
    ToastNotificationManager, ToastNotifier,
};
use windows::core::{HSTRING, IInspectable, Interface, h};

use super::store::{ToastEvent, ToastStore};
use super::{NotificationRequest, NotifyError, is_open_activation};

pub(super) const POWERSHELL_APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

struct LiveToast {
    app_id: HSTRING,
    tag: HSTRING,
    history_id: Option<u32>,
    notifier: ToastNotifier,
    notification: ToastNotification,
    activated: Option<i64>,
    dismissed: Option<i64>,
    on_click: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for LiveToast {
    fn drop(&mut self) {
        if let Some(token) = self.activated {
            let _ = self.notification.RemoveActivated(token);
        }
        if let Some(token) = self.dismissed {
            let _ = self.notification.RemoveDismissed(token);
        }
        // Expired or evicted entries cannot reopen the app. Use the original notifier/app id
        // to remove them from Notification Center too, after revoking their subscriptions.
        let _ = self.notifier.Hide(&self.notification);
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.RemoveGroupedTagWithId(&self.tag, h!("vibe-work"), &self.app_id);
        }
    }
}

// tauri-winrt-notification 0.8.1's Toast::show drops its ToastNotification on return, losing
// Activated before any click. Own it process-wide (also for updater/tray callers that return
// immediately). ToastNotification is agile: windows implements Send + Sync, so OS callback
// threads can safely remove it here. Handlers capture only an id, avoiding an ownership cycle.
fn store() -> MutexGuard<'static, ToastStore<LiveToast>> {
    static TOASTS: OnceLock<Mutex<ToastStore<LiveToast>>> = OnceLock::new();
    TOASTS
        .get_or_init(|| Mutex::new(ToastStore::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn event(id: u64, reason: ToastEvent, open: bool) {
    let pending = store().event(id, reason);
    if let Some(mut toast) = pending {
        // Releasing the callback may let a short-lived caller exit; finish Hide first.
        let click = toast.on_click.take();
        drop(toast);
        if let Some(click) = click.filter(|_| open) {
            click();
        }
    }
}

fn notification(request: &NotificationRequest) -> windows::core::Result<ToastNotification> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(h!(
        "<toast duration=\"short\"><visual><binding template=\"ToastGeneric\">\
         <text id=\"1\"/><text id=\"2\"/></binding></visual>\
         <audio silent=\"true\"/><actions><action content=\"Open\" arguments=\"default\"/>\
         </actions></toast>"
    ))?;
    let text = xml.GetElementsByTagName(h!("text"))?;
    // Text nodes preserve plain text (including XML metacharacters and Unicode).
    text.Item(0)?
        .AppendChild(&xml.CreateTextNode(&HSTRING::from(&request.title))?)?;
    text.Item(1)?
        .AppendChild(&xml.CreateTextNode(&HSTRING::from(&request.body))?)?;
    ToastNotification::CreateToastNotification(&xml)
}

pub(super) fn show_toast(
    app_id: &str,
    history_id: Option<u32>,
    request: &NotificationRequest,
    on_click: Box<dyn FnOnce() + Send>,
) -> Result<(), NotifyError> {
    show(app_id, history_id, request, on_click).map_err(|error| NotifyError(error.to_string()))
}

pub(super) fn dismiss(app_id: &str, history_id: u32) {
    let app_id = HSTRING::from(app_id);
    let removed = store()
        .remove_where(|toast| toast.app_id == app_id && toast.history_id == Some(history_id));
    drop(removed);
}

fn show(
    app_id: &str,
    history_id: Option<u32>,
    request: &NotificationRequest,
    on_click: Box<dyn FnOnce() + Send>,
) -> windows::core::Result<()> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;
    let notification = notification(request)?;
    let tag = HSTRING::from(format!("{id:x}"));
    notification.SetTag(&tag)?;
    notification.SetGroup(h!("vibe-work"))?;
    let mut live = LiveToast {
        app_id: HSTRING::from(app_id),
        tag,
        history_id,
        notifier: notifier.clone(),
        notification,
        activated: None,
        dismissed: None,
        on_click: Some(on_click),
    };
    live.activated = Some(live.notification.Activated(&TypedEventHandler::<
        ToastNotification,
        IInspectable,
    >::new(move |_, args| {
        let arguments = args
            .as_ref()
            .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|args| args.Arguments().ok())
            .filter(|args| !args.is_empty())
            .map(|args| args.to_string());
        event(
            id,
            ToastEvent::Activated,
            is_open_activation(arguments.as_deref()),
        );
        Ok(())
    }))?);
    live.dismissed = Some(live.notification.Dismissed(&TypedEventHandler::<
        ToastNotification,
        ToastDismissedEventArgs,
    >::new(move |_, args| {
        match args.as_ref().and_then(|args| args.Reason().ok()) {
            Some(ToastDismissalReason::UserCanceled) => event(id, ToastEvent::UserCanceled, false),
            Some(ToastDismissalReason::ApplicationHidden) => event(id, ToastEvent::Removed, false),
            // No COM activator is registered, so a saved Notification Center entry cannot open us.
            Some(ToastDismissalReason::TimedOut) => event(id, ToastEvent::TimedOut, false),
            _ => {}
        }
        Ok(())
    }))?);
    let shown = live.notification.clone();
    // Insert before Show: a callback may arrive before it returns. Never reinsert after an event.
    let evicted = store().insert(id, live);
    drop(evicted);
    let result = notifier.Show(&shown);
    if result.is_err() {
        event(id, ToastEvent::Removed, false);
    }
    result
}
