//! Checked vault captures: the source value and entry-wide permission commit in one sealed head.
//! Called only by the core capture broker; decisions bind an existing item and never overwrite a concurrent edit.
use crate::service::{blocking, now_ms};
use crate::{EntryPermission, KeyError, KeyService, ProjectSecret};
use std::cell::RefCell;
use vw_vault::items::{VaultItem, VaultResource};
use vw_vault::{SecretString, entry_permission, project_secret};

/// Value-free metadata from the committed head; receipt construction needs no further vault reads.
pub struct CaptureStored {
    /// Saved entry metadata.
    pub secret: ProjectSecret,
    /// Whether the person chose to replace an existing entry.
    pub replaced: bool,
    /// The entry-wide default in the committed head, preserving replacements' existing choice.
    pub permission: EntryPermission,
}
fn receipt(
    item: &VaultItem,
    items: &[VaultItem],
    replaced: bool,
    permission: EntryPermission,
) -> Option<CaptureStored> {
    let permission = if replaced {
        match items
            .iter()
            .find(|i| i.id == entry_permission::default_id_for(&item.id))
        {
            Some(policy) => {
                entry_permission::default_from_item(policy)
                    .filter(|p| p.entry_id == item.id)?
                    .permission
            }
            None => EntryPermission::AskEachTime,
        }
    } else {
        permission
    };
    Some(CaptureStored {
        secret: project_secret::agent_visible(item)?,
        replaced,
        permission,
    })
}

/// A checked name and all existing items with that name, retained only inside the broker.
pub struct CapturePlan {
    name: String,
    existing: Vec<VaultItem>,
}
fn name_of(item: &VaultItem) -> String {
    VaultResource::from_item(item).map_or_else(|| item.title.clone(), |r| r.label)
}
impl CapturePlan {
    /// The exact entry name bound to this preparation.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Whether any vault entry already has this name, including entries hidden from the agent.
    #[must_use]
    pub fn duplicate(&self) -> bool {
        !self.existing.is_empty()
    }

    /// Metadata for a single unambiguous agent-visible entry, with no value.
    #[must_use]
    pub fn existing(&self) -> Option<ProjectSecret> {
        if self.existing.len() != 1 {
            return None;
        }
        self.existing
            .first()
            .and_then(project_secret::agent_visible)
    }
}
/// The person's duplicate-name decision, or a new entry with an unused name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptureChoice {
    /// Create only when the requested name is still unused.
    New,
    /// Replace only the exact existing item the person approved.
    Replace,
    /// Keep the existing item and create a new entry with an unused numbered name.
    KeepBoth,
}
impl KeyService {
    /// Rechecks the settled destination before a source command can create anything.
    /// # Errors
    /// A locked vault or a concurrent change to the destination name or item.
    pub async fn validate_capture_plan(&self, plan: &CapturePlan) -> Result<(), KeyError> {
        let current = self.plan_capture(plan.name.clone()).await?;
        if current.existing != plan.existing {
            return Err(KeyError::InvalidSecret(
                "the vault entry changed; try again",
            ));
        }
        Ok(())
    }
    /// Settles a Keep both name before a command is launched, without writing any vault item.
    /// The final transaction refuses a new collision instead of silently choosing another name.
    /// # Errors
    /// A locked/unreadable vault or no unused name.
    pub async fn settle_capture_choice(
        &self,
        plan: CapturePlan,
        choice: CaptureChoice,
    ) -> Result<(CapturePlan, CaptureChoice), KeyError> {
        if choice != CaptureChoice::KeepBoth {
            return Ok((plan, choice));
        }
        let vault = self.vault();
        let name = blocking(move || {
            let items = vault.list_items_strict()?;
            (2..=10000)
                .map(|i| format!("{} ({i})", plan.name.chars().take(70).collect::<String>()))
                .find(|name| {
                    !items
                        .iter()
                        .any(|item| name_of(item).trim().to_lowercase() == name.to_lowercase())
                })
                .ok_or(KeyError::InvalidSecret("no unused capture name"))
        })
        .await??;
        let plan = self.plan_capture(name).await?;
        if plan.duplicate() {
            return Err(KeyError::InvalidSecret("the capture name changed"));
        }
        Ok((plan, CaptureChoice::New))
    }
    /// Checks a capture name and snapshots any existing item for later comparison.
    /// # Errors
    /// Invalid names, unreadable or locked vaults.
    pub async fn plan_capture(&self, name: String) -> Result<CapturePlan, KeyError> {
        if !project_secret::valid_name(&name) {
            return Err(KeyError::InvalidSecret("invalid name"));
        }
        let vault = self.vault();
        blocking(move || {
            let items = vault.list_items_strict()?;
            let mut existing: Vec<_> = items
                .into_iter()
                .filter(|item| name_of(item).trim().to_lowercase() == name.trim().to_lowercase())
                .collect();
            existing.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(CapturePlan {
                name: name.trim().into(),
                existing,
            })
        })
        .await?
    }
    /// Stores a captured value with its global default; project exceptions are never changed.
    /// Existing entries retain their permissions when replaced.
    /// # Errors
    /// Empty/oversized values, a stale name or item, foreign replacement, or vault failure.
    pub async fn store_capture(
        &self,
        plan: CapturePlan,
        choice: CaptureChoice,
        value: SecretString,
        origin: Option<String>,
        permission: EntryPermission,
    ) -> Result<CaptureStored, KeyError> {
        if value.is_empty() || value.len() > project_secret::MAX_VALUE {
            return Err(KeyError::InvalidSecret(
                "the captured value cannot be stored",
            ));
        }
        value.register_as(vw_secrets::SecretKind::VaultValue);
        let vault = self.vault();
        blocking(move || {
            let replacement = choice == CaptureChoice::Replace;
            let existing = plan.existing();
            if replacement && existing.as_ref().is_none_or(|s| !s.own) {
                return Err(KeyError::NotOwnSecret);
            }
            let id = if replacement {
                existing.as_ref().ok_or(KeyError::NoSuchSecret)?.id.clone()
            } else {
                project_secret::new_id()
            };
            let stored = RefCell::new(None);
            let committed = vault.update_items_checked(|items| {
                let mut same: Vec<_> = items
                    .iter()
                    .filter(|item| name_of(item).trim().to_lowercase() == plan.name.to_lowercase())
                    .collect();
                same.sort_by(|a, b| a.id.cmp(&b.id));
                if !same.into_iter().eq(plan.existing.iter()) {
                    return None;
                }
                if choice == CaptureChoice::New && !plan.existing.is_empty() {
                    return None;
                }
                let name = if choice == CaptureChoice::KeepBoth {
                    (2..=10000)
                        .map(|i| {
                            format!("{} ({i})", plan.name.chars().take(70).collect::<String>())
                        })
                        .find(|name| {
                            !items.iter().any(|item| {
                                name_of(item).trim().to_lowercase() == name.to_lowercase()
                            })
                        })?
                } else {
                    plan.name.clone()
                };
                let now = now_ms();
                let mut resource = project_secret::resource(
                    &id,
                    &name,
                    &value,
                    origin.as_deref(),
                    existing.as_ref().and_then(|s| s.label),
                    plan.existing
                        .first()
                        .filter(|_| replacement)
                        .map(|i| i.created_ms),
                    now,
                );
                resource
                    .public_fields
                    .insert("preserveWhitespace".into(), serde_json::json!(true));
                let mut changed = vec![resource.to_item().ok()?];
                if !replacement {
                    if items.iter().any(|i| i.id == id) {
                        return None;
                    }
                    let default = entry_permission::DefaultPolicy {
                        id: entry_permission::default_id_for(&id),
                        entry_id: id.clone(),
                        permission,
                        changed_ms: now,
                        created_ms: now,
                    };
                    if items.iter().any(|i| i.id == default.id) {
                        return None;
                    }
                    changed.push(
                        entry_permission::default_resource(&default)
                            .to_item()
                            .ok()?,
                    );
                }
                *stored.borrow_mut() = Some(receipt(&changed[0], items, replacement, permission)?);
                Some(changed)
            })?;
            if !committed {
                return Err(KeyError::InvalidSecret(
                    "the vault entry changed; try again",
                ));
            }
            stored.into_inner().ok_or(KeyError::Internal)
        })
        .await?
    }
}
