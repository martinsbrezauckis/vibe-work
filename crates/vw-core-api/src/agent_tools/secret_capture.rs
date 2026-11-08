//! Saves a command's stdout to the vault without returning or caching the output.
//! The authenticated hook settles name/consent before launch; the private executor and checked vault commit finish it.
use crate::{
    ask::tool_calls::{receipts::Execution, vault_save},
    core::Shared,
};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_agent_tools::{secretspec, wire::tools as names};
use vw_keys::{CaptureChoice, EntryPermission};

fn current(shared: &Shared, owner: &str, stop: u64, permission: EntryPermission) -> bool {
    crate::stops::generation(shared, owner) == stop
        && crate::agent_access::check(shared).is_ok()
        && super::vault_capture::permission(shared, owner).ok() == Some(permission)
        && shared.store.view(owner).is_some()
}

fn input(args: &Value) -> Result<(&str, bool, secretspec::RunSpec), String> {
    let fields = args
        .as_object()
        .ok_or("A command capture needs name and program; supply a command to run.")?;
    if fields.keys().any(|k| {
        ![
            "name",
            "program",
            "args",
            "cwd",
            "env",
            "timeoutSeconds",
            "format",
        ]
        .contains(&k.as_str())
    }) {
        return Err("A command capture accepts only a name and command options; do not supply a secret value.".into());
    }
    let name = args["name"]
        .as_str()
        .filter(|n| vw_vault::project_secret::valid_name(n))
        .ok_or("The vault name is invalid; choose a short entry name.")?;
    let single_line = match args.get("format").and_then(Value::as_str) {
        None if !fields.contains_key("format") => false,
        Some("exact") => false,
        Some("singleLine") => true,
        _ => return Err("The output format is invalid; use exact or singleLine.".into()),
    };
    let spec = secretspec::parse_capture(args)?;
    Ok((name, single_line, spec))
}

pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    args: &Value,
) -> Result<Execution, String> {
    crate::agent_access::check_for(shared, vault_save::KIND, owner)?;
    let (name, single_line, spec) = input(args)?;
    super::secret_run::check_inputs(shared, owner, &spec).await?;
    let permission = super::vault_capture::permission(shared, owner)?;
    let stop = crate::stops::generation(shared, owner);
    let keys = shared
        .config
        .keys
        .as_ref()
        .ok_or("No vault is connected; ask the user to set up the vault.")?;
    let plan = keys
        .plan_capture(name.into())
        .await
        .map_err(|e| super::vault_capture::vault_error(shared, &e))?;
    super::vault_capture::check_scope(shared, owner, &plan)?;
    let choice = if plan.duplicate() || permission != EntryPermission::Allowed {
        let command = super::secret_run::command_text(&spec.program, &spec.args);
        let replace = super::vault_capture::replace_allowed(shared, owner, keys, &plan).await?;
        vault_save::ask(
            shared,
            owner,
            name,
            &command,
            plan.duplicate(),
            replace,
            names::SECRET_CAPTURE_COMMAND,
        )
        .await?
    } else {
        CaptureChoice::New
    };
    let (plan, choice) = keys
        .settle_capture_choice(plan, choice)
        .await
        .map_err(|e| super::vault_capture::vault_error(shared, &e))?;
    let command = super::secret_run::prepare_capture(shared, owner, args, spec).await?;
    let owner = owner.to_owned();
    let shared = Arc::downgrade(shared);
    Ok(Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed; try again.")?;
        if !current(&shared, &owner, stop, permission) {
            return Err(
                "The capture ended or its access changed; ask the user before starting it again."
                    .into(),
            );
        }
        let keys = shared
            .config
            .keys
            .as_ref()
            .ok_or("The vault is unavailable; try again.")?;
        // Check the locked state again before launching; the original plan still binds the final transaction.
        keys.validate_capture_plan(&plan)
            .await
            .map_err(|e| super::vault_capture::vault_error(&shared, &e))?;
        if choice == CaptureChoice::Replace
            && !super::vault_capture::replace_allowed(&shared, &owner, keys, &plan).await?
        {
            return Err("The entry's access changed; request the save again.".into());
        }
        let values = command.values().await?;
        let value = super::secret_capture_exec::execute(
            &shared,
            &command.exec(),
            &values,
            single_line,
            || current(&shared, &owner, stop, permission),
        )
        .await?;
        command.check().await?;
        if plan.name().contains(value.expose()) {
            return Err(
                "The entry name contains the captured value; choose a different name.".into(),
            );
        }
        if choice == CaptureChoice::Replace
            && !super::vault_capture::replace_allowed(&shared, &owner, keys, &plan).await?
        {
            return Err("The entry's access changed; request the save again.".into());
        }
        if !current(&shared, &owner, stop, permission) {
            return Err(
                "The capture ended or its access changed; ask the user before starting it again."
                    .into(),
            );
        }
        let stored = keys
            .store_capture(plan, choice, value, None, permission)
            .await
            .map_err(|e| super::vault_capture::vault_error(&shared, &e))?;
        shared.emit(crate::CoreEvent::SecretsChanged {});
        Ok(json!({"name":stored.secret.name,"source":"command",
            "permission":vw_vocabulary::label("vaultPermissions", stored.permission.as_str())}))
    }))
}
