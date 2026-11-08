//! Commits a small set of sealed items after checking the same head's complete item list.
//! Used by direct secret capture so value and default permission appear together and names cannot race.
use super::{CommitOutcome, LocalRepo, MasterKey, Snapshot, VaultError, VaultItem};

pub(super) fn commit(
    repo: &LocalRepo,
    master: &MasterKey,
    device: &str,
    change: impl Fn(&[VaultItem]) -> Option<Vec<VaultItem>>,
) -> Result<bool, VaultError> {
    let lock = repo.lock()?;
    for _ in 0..super::MAX_COMMIT_ATTEMPTS {
        let head = super::head_snapshot(repo, master)?;
        let (parent, mut files) = match &head {
            Some((head, snapshot)) => (Some(head.id), snapshot.files.clone()),
            None => (None, Vec::new()),
        };
        let mut current = Vec::new();
        for file in files
            .iter()
            .filter(|f| f.path.starts_with(super::VAULT_PREFIX))
        {
            let item = super::decode(&super::entry_bytes(repo, master, file)?)?;
            if file.path != super::item_path(&item.id) {
                return Err(VaultError::Corrupt("invalid item path".into()));
            }
            current.push(item);
        }
        let Some(next) = change(&current) else {
            return Ok(false);
        };
        for item in &next {
            if !super::valid_item_id(&item.id) {
                return Err(VaultError::Corrupt("invalid item id".into()));
            }
            let entry = super::seal_item(repo, master, item)?;
            files.retain(|f| f.path != entry.path);
            files.push(entry);
        }
        let sealed = Snapshot::new(parent, device, super::now_ms(), files).seal(master);
        match repo.commit_if_head_locked(&lock, head.as_ref().map(|(h, _)| h), &sealed)? {
            CommitOutcome::Committed(_) => return Ok(true),
            CommitOutcome::HeadMoved => {}
        }
    }
    Err(VaultError::Conflict {
        attempts: super::MAX_COMMIT_ATTEMPTS,
    })
}
