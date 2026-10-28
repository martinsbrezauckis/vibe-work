//! Current vault permission, native consent and secret execution.
//! Depends on the broker job and vault policies; the hook checks consent before the command starts.
use super::{
    Approval, Confirmation, Confirmer, Duration, Job, SecretString, SecretUseView, Value, json,
    keys, now_ms,
};

pub(super) struct Permission {
    policies: std::collections::BTreeMap<String, crate::entry_permissions::Policy>,
    approval: Approval,
}

impl Job {
    fn log(&self, decision: &str) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let session = shared.names.display_title(&shared.store, &self.session_id);
        shared.secret_uses.log(SecretUseView {
            at_ms: now_ms(),
            session,
            project: crate::secret_path::display(&self.project).into_owned(),
            program: crate::secret_path::display(&self.program.to_string_lossy()).into_owned(),
            bindings: self
                .bound
                .iter()
                .map(|b| format!("{} <- {}", b.env, b.secret.name))
                .collect(),
            decision: decision.to_owned(),
        });
        shared.emit(crate::event::CoreEvent::SecretsChanged {});
    }

    async fn consent(&self, all_allowed: bool) -> Result<Approval, String> {
        if all_allowed {
            return Ok(Approval::EntryAllowed);
        }
        let mut dialog = self.dialog.clone();
        dialog.params["sessionId"] = json!(self.session_id);
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        match shared.confirmer.confirm(&dialog).await {
            Confirmation::Approved => Ok(Approval::Once),
            Confirmation::ApprovedAlways => Ok(Approval::Always),
            answer @ Confirmation::Declined => {
                self.log("declined");
                Err(answer.agent_text("this use of a secret"))
            }
        }
    }

    pub(super) async fn permission(&self) -> Result<Permission, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        let keys = keys(&shared)?;
        let mut policies = std::collections::BTreeMap::new();
        for (index, binding) in self.bound.iter().enumerate() {
            let policy = self
                .policy_project
                .policy(keys, &binding.secret.id)
                .await
                .map_err(|_| {
                    "Not run: this entry's project permission could not be checked".to_owned()
                })?;
            if policy.permission == vw_keys::EntryPermission::Never {
                return Err(super::missing_secret(&self.spec.bindings[index].secret));
            }
            policies.insert(binding.secret.id.clone(), policy);
        }
        let all_allowed = policies
            .values()
            .all(|policy| policy.permission == vw_keys::EntryPermission::Allowed);
        let approval = self.consent(all_allowed).await?;
        // A changed choice while the person was deciding cannot be overwritten by their older window.
        for (id, policy) in &mut policies {
            if self.policy_project.policy(keys, id).await.ok().as_ref() != Some(policy) {
                self.log("refused");
                return Err(
                    "Not run: an entry permission changed meanwhile; ask the user to try again"
                        .to_owned(),
                );
            }
            if matches!(approval, Approval::Always) {
                *policy = self.policy_project.allow(keys, id, policy).await?;
            }
        }
        Ok(Permission { policies, approval })
    }

    pub(super) async fn run(self, permission: Permission) -> Result<Value, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        let keys = keys(&shared)?;
        let Permission { policies, approval } = permission;
        crate::agent_access::check(&shared)?;
        let mut values: Vec<(String, SecretString)> = Vec::new();
        for b in &self.bound {
            let value = keys
                .secret_value(b.secret.id.clone(), b.field.clone())
                .await
                .map_err(|_| {
                    format!("Not run: the secret '{}' could not be read", b.secret.name)
                })?;
            values.push((b.env.clone(), value));
        }

        for (id, policy) in &policies {
            if self.policy_project.policy(keys, id).await.ok().as_ref() != Some(policy) {
                self.log("refused");
                return Err(
                    "Not run: an entry permission changed meanwhile; ask the user to try again"
                        .to_owned(),
                );
            }
        }
        crate::agent_access::check(&shared)?;
        self.log(approval.as_str());
        let mut result = super::super::secret_exec::execute(
            &shared,
            &super::super::secret_exec::Exec {
                process_env: &self.env,
                session_id: &self.session_id,
                program: &self.program,
                args: &self.spec.args,
                cwd: &self.cwd,
                timeout: Duration::from_secs(self.spec.timeout_s),
            },
            &values,
        )
        .await?;
        result["secretsUsed"] = json!(
            self.bound
                .iter()
                .map(|b| json!({"variable": b.env, "secret": b.secret.name}))
                .collect::<Vec<_>>()
        );
        Ok(result)
    }
}
