//! Log lines for the startup config steps ([`crate::startup_config`]), one per step outcome after the combined
//! write. A step whose edit was part of a failed write logs the error; a refused write logs its reason as the
//! step's "config not changed" line. None of these fail the launch.

use crate::commit_signature::CommitSignature;
use crate::experiment_pins::ExperimentPins;
use crate::title_model::TitleModel;
use crate::trust_store::TrustStore;

/// The credit-line step ([`crate::commit_signature`]).
pub(crate) fn commit_signature(outcome: Result<CommitSignature, String>) {
    match outcome {
        Ok(CommitSignature::Added) => {
            tracing::info!("agent commits: credit line off (include_commit_signature = false)");
        }
        Ok(CommitSignature::Kept(on)) => tracing::info!(
            on,
            "agent commits: config sets include_commit_signature, kept"
        ),
        Ok(CommitSignature::Skipped(why)) => {
            tracing::warn!(%why, "agent commits: config not changed");
        }
        Err(error) => tracing::warn!(%error, "agent commits: config.toml could not be updated"),
    }
}

/// The trust-store step ([`crate::trust_store`]). Without it Vibe falls back to its bundled certificate list.
pub(crate) fn trust_store(outcome: Result<TrustStore, String>) {
    match outcome {
        Ok(TrustStore::Added) => {
            tracing::info!(
                "TLS trust: system certificate store on (enable_system_trust_store = true)"
            );
        }
        Ok(TrustStore::Kept(true)) => {
            tracing::info!("TLS trust: config sets enable_system_trust_store = true, kept");
        }
        Ok(TrustStore::Kept(false)) => tracing::warn!(
            "TLS trust: config sets enable_system_trust_store = false, kept (Vibe uses its bundled certificate list)"
        ),
        Ok(TrustStore::Skipped(why)) => {
            tracing::warn!(%why, "TLS trust: config not changed");
        }
        Err(error) => tracing::warn!(%error, "TLS trust: config.toml could not be updated"),
    }
}

/// The experiment step ([`crate::experiment_pins`]), by key name. Invalid config remains for Vibe to diagnose.
pub(crate) fn experiment_pins(outcome: Result<ExperimentPins, String>) {
    match outcome {
        Ok(outcome) => {
            if let Some(why) = &outcome.skipped {
                tracing::warn!(%why, "experiments: config not changed");
            }
            for key in &outcome.added {
                tracing::info!(key, "experiments: pinned off");
            }
            for key in &outcome.disabled {
                tracing::info!(key, "experiments: unsupported feature pinned off");
            }
        }
        Err(error) => tracing::warn!(%error, "experiments: config.toml could not be updated"),
    }
}

/// The session-title step ([`crate::title_model`]). Without it sessions may only miss their automatic title.
pub(crate) fn title_model(outcome: Result<TitleModel, String>) {
    match outcome {
        Ok(outcome @ TitleModel::Skipped(_)) => tracing::warn!("{}", outcome.log_line()),
        Ok(outcome) => tracing::info!("{}", outcome.log_line()),
        Err(error) => tracing::warn!(%error, "session titles: config.toml could not be updated"),
    }
}
