//! A saved `large-4` choice is settled against the chat's own model list, which can contain a project's model under
//! that alias. When the list lacks the alias and the choice names the retired app seed, it resolves to the one catalog
//! Large 4 row (`model_check::catalog_large_4`) with the chat's thinking level. A matching Vibe session pin without
//! a saved name also resolves there. Other unavailable models keep the usual fallback and notice.
//! Called only with the chat's own model list: `restore` (reopen), `live::before` (Send), and a
//! stored chat's view read for its folder. The saved record is rewritten once at reopen or Send.
use super::{Choice, Records};
use crate::model_check::{OWN_ALIAS, catalog_large_4};
use serde_json::Value;

/// The model of the retired app seed's row (`vw_host::default_model`: `name = "mistral-large-4"` under
/// `alias = "large-4"`); every choice the app recorded on that row carries it.
const SEED_NAME: &str = "mistral-large-4";

/// `choice` on the catalog Large 4 when it names the retired alias that `runtime`'s model list does not offer.
/// A chat whose level was never set (read as Off) gets the catalog model's own level when it offers no Off.
/// `runtime` must be the chat's own model list (reopen, Send, a stored chat's view read for its folder): a choice
/// that names no model (Vibe's own session pin) is settled by that list, which offers a project's own `large-4`.
pub(super) fn resolve(runtime: &Value, choice: &Choice) -> Option<Choice> {
    // A choice that names another model under the alias (a project's own `large-4` row, e.g. a private provider's
    // model) is that model, never Mistral's Large 4, whichever list is read.
    if choice.alias != OWN_ALIAS || !(choice.name.is_empty() || choice.name == SEED_NAME) {
        return None;
    }
    let config = runtime.get("config")?;
    // Without a model list nothing shows the alias left it; a row of the person's own keeps the alias.
    if config["models"]
        .as_array()?
        .iter()
        .any(|row| row["alias"] == OWN_ALIAS)
    {
        return None;
    }
    let row = catalog_large_4(config)?;
    let mut effort = choice.effort.clone();
    if effort == "off"
        && let Some(levels) = row["thinkingLevels"].as_array()
        && !levels.iter().any(|level| level == "off")
        && let Some(own) = row["thinking"].as_str().filter(|own| !own.is_empty())
    {
        effort = own.into();
    }
    Some(Choice {
        alias: row["alias"].as_str()?.into(),
        name: row["name"].as_str().unwrap_or_default().into(),
        display_name: row["displayName"].as_str().unwrap_or_default().into(),
        effort,
    })
}

/// Rewrites the chat's saved choices that name the retired alias, once; a record without one is left untouched.
pub(super) fn migrate(records: &Records, id: &str, runtime: &Value) {
    let before = records.get(id);
    let fields = [&before.pending, &before.remembered, &before.fallback];
    if !fields
        .into_iter()
        .flatten()
        .any(|choice| resolve(runtime, choice).is_some())
    {
        return;
    }
    if let Err(error) = records.update(id, |record| {
        for field in [
            &mut record.pending,
            &mut record.remembered,
            &mut record.fallback,
        ] {
            if let Some(resolved) = field.as_ref().and_then(|choice| resolve(runtime, choice)) {
                *field = Some(resolved);
            }
        }
    }) {
        tracing::warn!(%error, "a chat's saved Large 4 choice could not be updated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn choice(alias: &str, effort: &str) -> Choice {
        Choice {
            alias: alias.into(),
            effort: effort.into(),
            display_name: String::new(),
            name: String::new(),
        }
    }

    fn row(alias: &str, name: &str, source: &str, label: &str) -> Value {
        json!({"alias":alias,"name":name,"source":source,"displayName":label,
            "thinking":"high","thinkingLevels":["low","medium","high"]})
    }

    fn runtime(rows: &[Value]) -> Value {
        json!({"config":{"models":rows}})
    }

    /// A chat record's identity and its own model list decide: another model under `large-4` stays put; the retired
    /// app seed resolves when absent. A pin without a saved name also uses the chat's own list.
    /// PREDICT (written before the earlier model-identity change): alias-only resolution moved a private model.
    #[test]
    fn a_project_model_saved_under_the_alias_is_not_the_retired_app_seed() {
        let chat = runtime(&[row(
            "mistral-large-4-0",
            "mistral-large-4-0",
            "catalog",
            "Mistral Large 4",
        )]);
        let named = |name: &str| Choice {
            name: name.into(),
            ..choice("large-4", "high")
        };
        let project = named("private-local-model");
        let migrated = resolve(&chat, &project);
        println!(
            "PROJECT_CHOICE name={} alias={}; MIGRATED={}",
            project.name,
            project.alias,
            if migrated.is_none() { "None" } else { "Some" }
        );
        assert!(migrated.is_none(), "project model moved");
        let seed = named("mistral-large-4");
        assert_eq!(resolve(&chat, &seed).unwrap().alias, "mistral-large-4-0");
        let alias_only = choice("large-4", "high");
        assert_eq!(
            resolve(&chat, &alias_only).unwrap().alias,
            "mistral-large-4-0"
        );
    }

    #[test]
    fn only_the_retired_alias_resolves_to_one_offered_catalog_large_4() {
        let large = row(
            "mistral-large-4-0",
            "mistral-large-4-0",
            "catalog",
            "Mistral Large 4",
        );
        let medium = row(
            "mistral-medium-latest",
            "mistral-medium-latest",
            "catalog",
            "Mistral Medium 3.5",
        );
        let offered = runtime(&[large.clone(), medium.clone()]);
        let resolved = resolve(&offered, &choice("large-4", "medium")).unwrap();
        assert_eq!(
            (
                resolved.alias.as_str(),
                resolved.name.as_str(),
                resolved.display_name.as_str(),
                resolved.effort.as_str()
            ),
            (
                "mistral-large-4-0",
                "mistral-large-4-0",
                "Mistral Large 4",
                "medium"
            )
        );
        // A level never set (read as Off) takes the model's own level; Large 4 offers no Off.
        assert_eq!(
            resolve(&offered, &choice("large-4", "off")).unwrap().effort,
            "high"
        );
        // Every other model, gone or not, is not ours to resolve.
        assert!(resolve(&offered, &choice("retired-flash", "high")).is_none());
        assert!(resolve(&offered, &choice("mistral-medium-latest", "high")).is_none());
        // The person's own `large-4` row is still offered: the choice stays as it is.
        let own = row("large-4", "mistral-large-4", "local", "My Large 4");
        assert!(resolve(&runtime(&[own, large.clone()]), &choice("large-4", "high")).is_none());
        // Two catalog Large 4 ids, a preview only, a local-only row or no list: no guess.
        let other = row(
            "mistral-large-4",
            "mistral-large-4",
            "catalog",
            "Mistral Large 4 (2511)",
        );
        let preview = row(
            "mistral-large-4-0",
            "mistral-large-4-0",
            "catalog",
            "Mistral Large 4 Preview",
        );
        let local = row(
            "mistral-large-4-0",
            "mistral-large-4-0",
            "local",
            "Mistral Large 4",
        );
        for view in [
            runtime(&[large, other]),
            runtime(&[preview, medium.clone()]),
            runtime(&[local, medium]),
            runtime(&[]),
            json!({"config":{"models":{}}}),
            json!({"config":{}}),
            json!({}),
        ] {
            assert!(
                resolve(&view, &choice("large-4", "high")).is_none(),
                "{view}"
            );
        }
    }
}
