//! Removes expired admission from cached hits and live redaction reports before worker operations.
//! Depends on filtered Vault inputs; the worker calls it for scans, manual actions and pending rescans.

use super::{Worker, finding_id};
use crate::{SecretLeakState, core::Shared};
use std::collections::BTreeSet;
use vw_keys::VaultScanSecret;

impl Worker {
    pub(super) fn restrict_inputs(&mut self, shared: &Shared, secrets: &[VaultScanSecret]) {
        let allowed: BTreeSet<_> = secrets.iter().map(|secret| secret.id.clone()).collect();
        for (path, file) in &mut self.cache.files {
            if file.hits.keys().all(|id| allowed.contains(id)) {
                continue;
            }
            file.hits.retain(|id, _| allowed.contains(id));
            if let Some(reports) = self.reports.get_mut(&finding_id(&file.chat_id))
                && let Some(report) = reports.get_mut(path)
            {
                *report = report.select_secrets(&allowed);
                file.places = super::count(report.place_count());
            } else {
                // A cold cache has no ranges with which to recompute overlapping places safely.
                file.hits.clear();
                file.places = 0;
            }
            file.fingerprint.clear();
        }
        self.cache.findings.retain(|finding| {
            finding.state == SecretLeakState::Removed
                || self
                    .cache
                    .files
                    .values()
                    .any(|file| file.chat_id == finding.chat_id && !file.hits.is_empty())
        });
        self.refresh(shared);
    }
}
