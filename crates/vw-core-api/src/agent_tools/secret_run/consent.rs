//! Current vault permission, native consent and secret execution, on the shared permission gate (`consent_gate`).
//! Depends on the broker job and vault policies; the hook checks consent before the command starts.
use super::{
    Approval, Confirmation, Confirmer, Duration, Job, SecretString, SecretUseView, Value, json,
    keys, now_ms,
};

/// The consent this job holds: the shared gate's granted record (each bound entry's current permission policy) and
/// the person's answer.
pub(super) type Permission = crate::consent_gate::Granted<Job>;

/// Each bound entry's permission policy, read now.
type Policies = std::collections::BTreeMap<String, crate::entry_permissions::Policy>;

const CHANGED: &str = "Not run: an entry permission changed meanwhile; ask the user to try again";

/// The checks of a command run with a secret on the shared permission gate (`consent_gate`): the same steps as a
/// connected tool's key, with the entries' project permissions as the record.
impl crate::consent_gate::Policy for Job {
    type Record = Policies;
    type Answer = Approval;
    type Refusal = String;
    type Value = Vec<(String, SecretString)>;

    /// Agent access is not paused, the request's secrets are still in scope, and no bound entry is Never.
    async fn check(&self) -> Result<Policies, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        crate::agent_access::check(&shared)?;
        self.check_scope()?;
        let keys = keys(&shared)?;
        let mut policies = Policies::new();
        for (index, binding) in self.bound.iter().enumerate() {
            let policy = self
                .policy_project
                .policy(keys, &binding.secret.id)
                .await
                .map_err(|_| {
                    "Not run: this entry's project permission could not be checked".to_owned()
                })?;
            if policy.permission == vw_keys::EntryPermission::Never {
                // Before the request was first checked, a Never entry is one the agent cannot see; afterwards it
                // was set while the person decided or before the run.
                if self.checked.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return Err(self.changed());
                }
                return Err(super::missing_secret(&self.spec.bindings[index].secret));
            }
            policies.insert(binding.secret.id.clone(), policy);
        }
        self.checked
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(policies)
    }

    /// Every entry Allowed, or the person's yes to this request.
    fn permits(&self, policies: &Policies, answer: Option<&Approval>) -> bool {
        answer.is_some()
            || policies
                .values()
                .all(|policy| policy.permission == vw_keys::EntryPermission::Allowed)
    }

    /// A changed choice while the person was deciding cannot be overwritten by their older window.
    fn same_as_shown(&self, shown: &Policies, now: &Policies) -> bool {
        shown == now
    }

    /// "Allowed" in the dialog makes each entry Allowed for this project.
    async fn confirmed(
        &self,
        mut policies: Policies,
        answer: &Approval,
    ) -> Result<Policies, String> {
        if matches!(answer, Approval::Always) {
            let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
            let keys = keys(&shared)?;
            for (id, policy) in &mut policies {
                *policy = self.policy_project.allow(keys, id, policy).await?;
            }
        }
        Ok(policies)
    }

    fn not_allowed(&self) -> String {
        self.changed()
    }

    fn changed(&self) -> String {
        self.log("refused");
        CHANGED.to_owned()
    }

    /// The bound secrets' values (the record was checked just before by the gate).
    async fn value(&self, _granted: &Policies) -> Result<Vec<(String, SecretString)>, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        let keys = keys(&shared)?;
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
        Ok(values)
    }
}

impl Job {
    pub(super) fn log(&self, decision: &str) {
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

    /// The native dialog for a request without a standing grant.
    async fn consent(&self) -> Result<Approval, String> {
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

    /// Steps 1 and 2 of the shared gate: a standing grant (every entry Allowed) or the person's answer.
    pub(super) async fn permission(&self) -> Result<Permission, String> {
        crate::consent_gate::grant(self, |_| self.consent()).await
    }

    /// How the run was allowed, for the use log.
    pub(super) fn approval(permission: &Permission) -> Approval {
        permission
            .answer()
            .copied()
            .unwrap_or(Approval::EntryAllowed)
    }

    /// The values for a run later than the consent: the record is checked again first and after the read.
    pub(super) async fn checked_values(
        &self,
        permission: &Permission,
    ) -> Result<Vec<(String, SecretString)>, String> {
        crate::consent_gate::read(self, permission)
            .await
            .map(|(_, values)| values)
    }

    /// The consent still holds on a record read now.
    pub(super) async fn check_permission(&self, permission: &Permission) -> Result<(), String> {
        crate::consent_gate::recheck(self, permission)
            .await
            .map(|_| ())
    }

    pub(super) async fn run(self, permission: Permission) -> Result<Value, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        let values = self.checked_values(&permission).await?;
        self.log(Self::approval(&permission).as_str());
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
