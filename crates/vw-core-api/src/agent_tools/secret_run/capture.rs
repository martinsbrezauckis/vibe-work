//! Reuses secret command admission and input-secret consent for the private stdout sink.
//! No ordinary command execution/result cache is called; the capture broker owns the returned preparation.
use super::{Arc, Job, Shared, Value, consent};
use vw_agent_tools::secretspec::RunSpec;
use vw_vault::SecretString;

pub(crate) struct CaptureCommand {
    job: Job,
    permission: consent::Permission,
}
pub(crate) async fn prepare_capture(
    shared: &Arc<Shared>,
    session: &str,
    args: &Value,
    spec: RunSpec,
) -> Result<CaptureCommand, String> {
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or("Tools are unavailable; reopen the chat.")?;
    let (_, job) = super::checked_job(shared, tools, session, args, spec).await?;
    let permission = job.permission().await?;
    Ok(CaptureCommand { job, permission })
}
impl CaptureCommand {
    pub(crate) fn exec(&self) -> super::super::secret_exec::Exec<'_> {
        super::super::secret_exec::Exec {
            process_env: &self.job.env,
            session_id: &self.job.session_id,
            program: &self.job.program,
            args: &self.job.spec.args,
            cwd: &self.job.cwd,
            timeout: std::time::Duration::from_secs(self.job.spec.timeout_s),
        }
    }
    pub(crate) async fn values(&self) -> Result<Vec<(String, SecretString)>, String> {
        let values = self.job.checked_values(&self.permission).await?;
        if !values.is_empty() {
            self.job.log(self.permission.approval.as_str());
        }
        Ok(values)
    }
    pub(crate) async fn check(&self) -> Result<(), String> {
        self.job.check_permission(&self.permission).await
    }
}
