//! Browser tab-close delegation receipts and page authorization checks.
//! Depends on the core authorizer and browser sessions; called by tab-close and native dispatch.

use std::sync::Arc;

use crate::delegation::{
    DelegatedAct, DelegationAuthority, DelegationAuthorizer, DelegationReceipt,
};
use crate::driver::Browser;
use crate::sessions::{Actor, Outcome, now_ms};

impl Browser {
    /// Retains a completed user-tab close in durable delegation history and the running session.
    ///
    /// # Errors
    /// The private receipt write failure; the close already happened and its receipt stays in memory.
    pub fn record_user_tab_close(
        &self,
        session_id: &str,
        site: &str,
        authority: DelegationAuthority,
    ) -> Result<(), String> {
        let receipt = DelegationReceipt {
            session_id: session_id.to_owned(),
            at_ms: now_ms(),
            site: site.to_owned(),
            act: DelegatedAct::CloseUserTabs,
            authority,
            outcome: Outcome::Done,
        };
        self.sessions.record_authorized(receipt, |receipt| {
            self.delegation_authorizer
                .read()
                .ok()
                .and_then(|slot| slot.clone())
                .ok_or_else(|| "browser receipt store is unavailable".to_owned())?
                .record(receipt)
        })
    }

    pub(crate) fn dispatch_permission_check(
        &self,
        page: &str,
        actor: Actor,
    ) -> Option<Arc<dyn Fn() -> bool + Send + Sync>> {
        if actor == Actor::User {
            return None;
        }
        let page = page.to_owned();
        let browser = self.clone();
        Some(Arc::new(move || {
            browser.delegation_authorizer.read().is_ok_and(|slot| {
                slot.as_ref()
                    .is_none_or(|authorizer| authorizer.page_authorized(&page, actor))
            })
        }))
    }

    /// Connects page admission and tab-close receipts to the core.
    pub fn set_delegation_authorizer(&self, authorizer: Arc<dyn DelegationAuthorizer>) {
        if let Ok(mut slot) = self.delegation_authorizer.write() {
            *slot = Some(authorizer);
        }
    }

    /// Deletes this session's durable delegation receipts when the core deletes the session.
    ///
    /// # Errors
    /// The private receipt storage failure.
    pub fn forget_delegation_receipts(&self, session_id: &str) -> Result<(), String> {
        let authorizer = self
            .delegation_authorizer
            .read()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(authorizer) = authorizer {
            authorizer.forget(session_id)?;
        }
        Ok(())
    }
}
