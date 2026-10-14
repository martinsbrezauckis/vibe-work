//! The head guard: a vault read never goes back to an older version, in this process.
//!
//! Why: the repository is plain files of the user (`repo`), and every process of the same user, the agent's commands
//! included, can change them. It cannot forge a record (every manifest is sealed with the master key), but it can
//! make an older sealed version the head again: delete the newest manifests (a rollback), or copy an older manifest
//! under the next generation number (a replay; the generation is only the file name). Either brings back a record
//! that was revoked since, e.g. an "always" grant of the secret broker or of an MCP server's key, without the unlock
//! material. ShellX Vault's sync engine guards the same for its remote copy (`vault-client/src/engine.rs:273-350`):
//! it refuses a head older than its last synced base and checks every parent link from the base forward.
//!
//! What this guard does: the [`ChainGuard`] (one per `Vault`, shared by every session of it) remembers the newest
//! head it has accepted (generation and identity, per repository folder). Reading the head and checking it happen
//! under one lock, so the reads of this process see heads in order. A head is accepted when it is the known one,
//! or a newer generation whose chain links back to it: every generation in between exists and its sealed `parent`
//! is the BLAKE3 of the previous sealed manifest (`vault_core::Snapshot::sealed_id`). Anything else (an older
//! generation, the known generation with other bytes, a missing generation, a broken link, an empty repository after
//! a head) is refused with [`VaultError::Corrupt`], so every read and write of the vault fails closed.
//!
//! Limits: the first head this process reads is the baseline, so a version replayed
//! while Vibe Work was not running is accepted at the next start; a prune that removed generations above the
//! known head while Vibe Work runs is refused (restart after a prune).
//!
//! Depends on: vault-core (`Snapshot`, `MasterKey`), `repo`, `error`. Called by: `items::head_snapshot` (through a
//! guarded [`LocalRepo`]), `vault` (one guard per vault).
//! Invariant: the known head only moves forward along a verified chain.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use vault_core::{MasterKey, Snapshot};

use crate::error::VaultError;
use crate::repo::{Head, LocalRepo};

/// The newest head accepted for one repository folder.
#[derive(Debug, Clone)]
struct Known {
    dir: PathBuf,
    generation: u64,
    id: [u8; 32],
}

/// Remembers the newest accepted head (module docs). `Default` = nothing seen yet.
#[derive(Debug, Default)]
pub struct ChainGuard {
    known: Mutex<Option<Known>>,
}

fn went_back(detail: &str) -> VaultError {
    VaultError::Corrupt(format!(
        "the vault went back to an older version or was changed outside a vault app ({detail}); if you restored \
         a backup or pruned the vault, restart Vibe Work"
    ))
}

impl ChainGuard {
    /// Reads `repo`'s head and opens it, accepting it only along the chain (module docs). `None` = empty repository
    /// that never had a head in this process.
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] for a refused head or a head that does not open with `master`, [`VaultError::Io`].
    pub(crate) fn head_snapshot(
        &self,
        repo: &LocalRepo,
        master: &MasterKey,
    ) -> Result<Option<(Head, Snapshot)>, VaultError> {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        if known.as_ref().is_some_and(|k| k.dir != repo.dir()) {
            *known = None;
        }
        let head = repo.head()?;
        let Some(head) = head else {
            return match known.as_ref() {
                None => Ok(None),
                Some(k) => Err(went_back(&format!(
                    "no version on disk after generation {}",
                    k.generation
                ))),
            };
        };
        let snapshot = open(master, &head.sealed)?;
        if let Some(k) = known.as_ref() {
            if head.generation < k.generation {
                return Err(went_back(&format!(
                    "generation {} after {}",
                    head.generation, k.generation
                )));
            }
            if head.generation == k.generation {
                if head.id != k.id {
                    return Err(went_back(&format!(
                        "generation {} was replaced",
                        head.generation
                    )));
                }
            } else {
                verify_links(repo, master, k, &head, &snapshot)?;
            }
        }
        *known = Some(Known {
            dir: repo.dir().to_path_buf(),
            generation: head.generation,
            id: head.id,
        });
        Ok(Some((head, snapshot)))
    }
}

fn open(master: &MasterKey, sealed: &[u8]) -> Result<Snapshot, VaultError> {
    Snapshot::open(master, sealed).map_err(|_| {
        VaultError::Corrupt("the latest vault version does not open with this key".to_owned())
    })
}

/// Every generation after `known` up to `head` exists and names the one before it as its parent.
fn verify_links(
    repo: &LocalRepo,
    master: &MasterKey,
    known: &Known,
    head: &Head,
    head_snapshot: &Snapshot,
) -> Result<(), VaultError> {
    let mut previous = known.id;
    for generation in known.generation + 1..head.generation {
        let sealed = repo
            .manifest(generation)?
            .ok_or_else(|| went_back(&format!("generation {generation} is missing")))?;
        if open(master, &sealed)?.parent != Some(previous) {
            return Err(went_back(&format!(
                "generation {generation} does not follow the one before"
            )));
        }
        previous = Snapshot::sealed_id(&sealed);
    }
    if head_snapshot.parent != Some(previous) {
        return Err(went_back(&format!(
            "generation {} does not follow the one before",
            head.generation
        )));
    }
    Ok(())
}
