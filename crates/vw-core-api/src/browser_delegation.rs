//! Browser tab-close permission and its durable receipt boundary.
//! Depends on browser delegation, private atomic storage and loaded session ownership.
//! Called by Browser settings, tab-close tools and browser startup.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use vw_browser::delegation::{BrowserPermissions, DelegationAuthorizer};

use crate::browser_view::BrowserResult;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

fn path(shared: &Shared) -> Result<PathBuf, CoreError> {
    shared
        .config
        .preview
        .as_ref()
        .map(|setup| setup.config.storage_root.join("browser-permissions.json"))
        .ok_or_else(|| CoreError::new(ErrorCode::Denied, "the browser is not available"))
}

/// Missing, corrupt or unreadable settings give no permission. Obsolete fields are ignored by the contract.
pub(crate) fn load(shared: &Shared) -> BrowserPermissions {
    path(shared)
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Reads or atomically stores the user/Debug API's browser tab-close permission.
pub(crate) fn settings(
    shared: &Shared,
    permissions: Option<BrowserPermissions>,
) -> Result<BrowserResult, CoreError> {
    let permissions = if let Some(permissions) = permissions {
        let path = path(shared)?;
        let bytes = serde_json::to_vec_pretty(&permissions)
            .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))?;
        vw_platform::replace_private(&path, &bytes)
            .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))?;
        shared.emit(crate::event::CoreEvent::BrowserChanged { session_id: None });
        permissions
    } else {
        path(shared)?;
        load(shared)
    };
    Ok(BrowserResult::Permissions { permissions })
}

struct NativeDelegation {
    shared: Weak<Shared>,
    receipts: crate::browser_delegation_receipts::ReceiptStore,
}

impl DelegationAuthorizer for NativeDelegation {
    fn page_authorized(&self, page_key: &str, actor: vw_browser::Actor) -> bool {
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let Some(hub) = shared.preview.as_ref() else {
            return false;
        };
        let Some(owner) = hub.service.owner_session_id(page_key) else {
            return false;
        };
        shared.store.view(&owner).is_some()
            && (actor != vw_browser::Actor::Agent
                || crate::browser_tabs::agent_page_allowed(&hub.service, &owner, page_key))
    }

    fn record(&self, receipt: &vw_browser::delegation::DelegationReceipt) -> Result<(), String> {
        let shared = self
            .shared
            .upgrade()
            .ok_or_else(|| "browser is closed".to_owned())?;
        let owner = shared
            .preview
            .as_ref()
            .and_then(|hub| hub.service.owner_session_id(&receipt.session_id))
            .ok_or_else(|| "browser page has no session owner".to_owned())?;
        let mut row = receipt.clone();
        row.session_id = owner;
        self.receipts.record(row)
    }

    fn forget(&self, session_id: &str) -> Result<(), String> {
        self.receipts.forget(session_id)
    }
}

/// Installs the tab-close receipt store on the browser.
pub(crate) fn install(shared: &Arc<Shared>) {
    if let Some(hub) = &shared.preview {
        let Ok(settings_path) = path(shared) else {
            return;
        };
        let receipts = crate::browser_delegation_receipts::ReceiptStore::load(
            settings_path.with_file_name("browser-delegation-receipts.json"),
        );
        hub.browser
            .sessions()
            .restore_delegation_receipts(receipts.all());
        hub.browser
            .set_delegation_authorizer(Arc::new(NativeDelegation {
                shared: Arc::downgrade(shared),
                receipts,
            }));
    }
}
