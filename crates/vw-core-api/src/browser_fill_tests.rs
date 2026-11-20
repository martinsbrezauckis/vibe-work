//! A prepared vault fill and a revoke that lands before it runs (`prepare_checked`, then the deferred write).
//!
//! A Chrome Control batch prepares every `auth.fill`/`auth.basic` step before its first step runs, and the built-in
//! browser's HTTP sign-in holds its prepared fill across the challenge: the person can revoke the site permission in
//! between. These tests drive the shared route (`prepare_checked` -> `Authorized::write`) with a scripted writer, a
//! real vault and the native `secrets.revoke` command, and check the writer and `KeyService::login_password`.
//!
//! PREDICT (written before the recheck): the deferred write rechecked only the entry policy, so in every revoked case
//! the writer is called once with the password and `password_reads` is 1; the unrevoked control fills either way.
//! The entry policy was read again only after the password: a permission changed from Allowed to Ask between prepare
//! and write refused the write but had already read the password (`password_reads` 1).

use std::sync::{Arc, Mutex};
use tokio::sync::watch;

use serde_json::json;
use vw_browser::{FillTarget, LoginRole};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, Placement, PreviewEngine,
    PreviewHooks, Storage,
};
use vw_vault::SecretString;

use super::{LoginWriter, prepare_checked};

const SITE: &str = "https://shop.example.test";
const PASSWORD: &str = "fixture-password-0123";
const CHAT: &str = "fill-chat";

/// The writer must refuse before it calls this engine after a completed grant revoke.
struct NoPageCalls;

fn no_page<T: Send + 'static>() -> EngineFuture<'static, T> {
    Box::pin(async { Err(vw_preview::EngineError::NotMounted) })
}

impl PreviewEngine for NoPageCalls {
    fn attach(&self, _: Arc<dyn PreviewHooks>) {}
    fn mount(&self, _: MountSpec) -> EngineFuture<'_, ()> {
        no_page()
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        no_page()
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        no_page()
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        no_page()
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        no_page()
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        no_page()
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        no_page()
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<std::path::PathBuf>> {
        no_page()
    }
}

/// The person's dialog answers, in order; a dialog nobody scripted fails the test.
struct Answers(Mutex<Vec<Confirmation>>);
impl Confirmer for Answers {
    fn confirm(
        &self,
        request: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        let next = self.0.lock().unwrap().pop();
        std::future::ready(next.unwrap_or_else(|| panic!("unexpected dialog: {}", request.method)))
    }
}

/// Records every value the route hands to the page.
#[derive(Clone, Default)]
struct Writer(Arc<Mutex<Vec<String>>>);
impl LoginWriter for Writer {
    async fn write(
        &self,
        _target: &FillTarget,
        _login: &vw_keys::Login,
        value: &str,
        effect: &super::FillEffect<'_>,
    ) -> Result<serde_json::Value, String> {
        let mut permit = effect.enter().await?;
        permit.dispatched();
        self.0.lock().unwrap().push(value.to_owned());
        permit.settled();
        Ok(json!({"filled": true}))
    }
}
impl Writer {
    fn values(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// PREDICT: a writer dispatched after the last grant check can pause before its page effect.
/// Without a shared effect lane, revoking during that pause lets the resumed writer put the
/// password into the page. The same grant covers prepared batch steps aimed at different frame refs.
#[derive(Clone)]
struct PausedWriter {
    dispatched: watch::Sender<usize>,
    resume: watch::Receiver<bool>,
    values: Writer,
}

impl PausedWriter {
    fn new() -> (Self, watch::Receiver<usize>, watch::Sender<bool>) {
        let (dispatched, seen) = watch::channel(0);
        let (resume, release) = watch::channel(false);
        (
            Self {
                dispatched,
                resume: release,
                values: Writer::default(),
            },
            seen,
            resume,
        )
    }
}

impl LoginWriter for PausedWriter {
    async fn write(
        &self,
        _target: &FillTarget,
        _login: &vw_keys::Login,
        value: &str,
        effect: &super::FillEffect<'_>,
    ) -> Result<serde_json::Value, String> {
        self.dispatched.send_modify(|n| *n += 1);
        let mut resume = self.resume.clone();
        resume.wait_for(|ready| *ready).await.unwrap();
        let mut permit = effect.enter().await?;
        permit.dispatched();
        self.values.0.lock().unwrap().push(value.to_owned());
        permit.settled();
        Ok(json!({"filled": true}))
    }
}

/// Holds the admitted page effect until the test allows its receipt to finish.
#[derive(Clone)]
struct AdmittedWriter {
    admitted: watch::Sender<bool>,
    resume: watch::Receiver<bool>,
    values: Writer,
}

impl LoginWriter for AdmittedWriter {
    async fn write(
        &self,
        _target: &FillTarget,
        _login: &vw_keys::Login,
        value: &str,
        effect: &super::FillEffect<'_>,
    ) -> Result<serde_json::Value, String> {
        let mut permit = effect.enter().await?;
        permit.dispatched();
        self.admitted.send_replace(true);
        let mut resume = self.resume.clone();
        resume.wait_for(|ready| *ready).await.unwrap();
        self.values.0.lock().unwrap().push(value.to_owned());
        permit.settled();
        Ok(json!({"filled": true}))
    }
}

/// A sign-in field (`auth.fill`) or an HTTP Basic challenge (`auth.basic`) on the fixture site.
fn target(basic: bool) -> FillTarget {
    FillTarget {
        reference: if basic { "dialog-1" } else { "e12" }.to_owned(),
        role: LoginRole::Password,
        origin: SITE.to_owned(),
        host: "shop.example.test".to_owned(),
        label: if basic { "HTTP sign-in" } else { "password" }.to_owned(),
    }
}

pub(crate) async fn memory_keys(root: &std::path::Path) -> Arc<vw_keys::KeyService> {
    vw_keys::KeyService::start(vw_keys::KeyServiceConfig {
        managed_vault: None,
        vault: vw_vault::VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(root.join("vault")),
            device_store: Arc::new(vw_vault::remember::MemoryDeviceStore::default()),
            kdf: vw_vault::KdfParams::default(),
            device_name: "test".into(),
        },
        proxy: vw_keyproxy::ProxyConfig::default(),
        test_api_base: "http://127.0.0.1:1".into(),
        settings_path: root.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: root.join(".env"),
            keyring: Arc::new(vw_keys::existing::MemoryVibeKeyring::default()),
        },
    })
    .await
    .unwrap()
}

struct Rig {
    core: crate::Core,
    keys: Arc<vw_keys::KeyService>,
    login: String,
    project: String,
    root: std::path::PathBuf,
}

impl Rig {
    /// A core with an unlocked vault holding one login for the site, and an open chat in a project folder.
    async fn new(name: &str, answers: Vec<Confirmation>) -> Self {
        Self::new_with_preview(name, answers, false).await
    }

    async fn new_with_preview(name: &str, answers: Vec<Confirmation>, preview: bool) -> Self {
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
            .join(format!("browser-fill-{name}-{}", std::process::id()));
        let keys = memory_keys(&root).await;
        keys.create_vault(SecretString::new("browser fill fixture passphrase"))
            .await
            .unwrap();
        let login = keys
            .store_login(
                "Shop".to_owned(),
                SITE.to_owned(),
                "alice".to_owned(),
                SecretString::new(PASSWORD),
            )
            .await
            .unwrap()
            .login
            .id;
        let mut answers = answers;
        answers.reverse();
        let core = crate::offline_core::start_with_confirmer(
            &format!("browser-fill-{name}"),
            |config, dir| {
                config.keys = Some(keys.clone());
                if preview {
                    config.preview = Some(crate::PreviewSetup {
                        engine: Arc::new(NoPageCalls),
                        config: vw_preview::PreviewConfig::new(dir.join("preview")),
                        guard: None,
                    });
                }
            },
            Answers(Mutex::new(answers)),
        )
        .await;
        // Git (the project identity check) runs only with Vibe Work's private hooks folder in place.
        vw_changes::prepare_root(&root.join("changes")).unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let shared = &core.shared;
        crate::entry_projects::before_open(
            shared,
            "session/start",
            &json!({"agentConfig": {"cwd": project}}),
        )
        .await
        .unwrap()
        .finish(shared, &json!({"state": {"session": {"id": CHAT}}}));
        let _ = shared
            .store
            .load(
                json!({"session": {"id": CHAT, "cwd": project, "workspaceRoots": [],
                    "status": {"type": "running"}}, "entries": [], "turns": []}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        let project = vw_platform::canonical_plain(&project)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        Self {
            core,
            keys,
            login,
            project,
            root,
        }
    }

    fn grant_id(&self) -> String {
        vw_vault::fill_grant::id_for(&self.login, SITE)
    }

    async fn add_chat(&self, chat: &str) {
        let project = self.root.join("project");
        crate::entry_projects::before_open(
            &self.core.shared,
            "session/start",
            &json!({"agentConfig": {"cwd": project}}),
        )
        .await
        .unwrap()
        .finish(
            &self.core.shared,
            &json!({"state": {"session": {"id": chat}}}),
        );
        let _ = self
            .core
            .shared
            .store
            .load(
                json!({"session": {"id": chat, "cwd": project, "workspaceRoots": [],
                    "status": {"type": "running"}}, "entries": [], "turns": []}),
                vw_session::LoadKind::New,
            )
            .unwrap();
    }

    /// The person's "Always for this site", saved before the batch.
    async fn saved_always(&self) {
        crate::fill_grants::grant_always(
            &self.core.shared,
            &self.login,
            "Shop",
            SITE,
            crate::fill_grants::now_ms(),
        )
        .await
        .unwrap();
    }

    /// The native `secrets.revoke` command (Settings > Project secrets, `secrets revoke`).
    async fn revoke(&self) {
        crate::secrets::execute(
            &self.core.shared,
            crate::SecretsRequest::Revoke {
                grant_id: self.grant_id(),
            },
        )
        .await
        .unwrap();
    }

    /// The native `secrets.setEntryPermission` command for the login in the chat's project.
    async fn permission(&self, permission: crate::EntryPermissionView) {
        crate::secrets::execute(
            &self.core.shared,
            crate::SecretsRequest::SetEntryPermission {
                project: self.project.clone(),
                id: self.login.clone(),
                permission,
            },
        )
        .await
        .unwrap();
    }

    /// Prepares one fill as an earlier batch step would leave it: decided, not yet written.
    async fn prepare(
        &self,
        basic: bool,
        writer: &Writer,
    ) -> crate::ask::tool_calls::receipts::Execution {
        self.prepare_with(target(basic), writer.clone()).await
    }

    async fn prepare_with<W: LoginWriter + Send + 'static>(
        &self,
        target: FillTarget,
        writer: W,
    ) -> crate::ask::tool_calls::receipts::Execution {
        self.prepare_in(CHAT, target, writer).await
    }

    async fn prepare_in<W: LoginWriter + Send + 'static>(
        &self,
        chat: &str,
        target: FillTarget,
        writer: W,
    ) -> crate::ask::tool_calls::receipts::Execution {
        prepare_checked(
            &self.core.shared,
            chat,
            "Shop",
            target,
            Requester::Agent,
            writer,
        )
        .await
        .expect("the fill is decided before the batch runs")
    }

    fn password_reads(&self) -> u64 {
        self.keys.login_counts().password_reads
    }

    async fn finish(self) {
        self.core.shutdown().await;
        self.keys.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn revoke_during_dispatched_batch_blocks_later_page_writes() {
    let rig = Rig::new("inflight-batch", Vec::new()).await;
    rig.saved_always().await;
    rig.add_chat("second-fill-chat").await;
    let (writer, mut dispatched, resume) = PausedWriter::new();
    let mut first = target(false);
    first.reference = "main-frame-field".to_owned();
    let mut second = target(false);
    second.reference = "child-frame-field".to_owned();
    let mut third = target(false);
    third.reference = "other-tab-field".to_owned();
    let a = tokio::spawn(rig.prepare_with(first, writer.clone()).await);
    let b = tokio::spawn(rig.prepare_with(second, writer.clone()).await);
    let c = tokio::spawn(
        rig.prepare_in("second-fill-chat", third, writer.clone())
            .await,
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        dispatched.wait_for(|n| *n == 3),
    )
    .await
    .expect("both prepared fills reached the writer")
    .unwrap();
    rig.revoke().await;
    resume.send_replace(true);
    let first_result = a.await.unwrap();
    let second_result = b.await.unwrap();
    let third_result = c.await.unwrap();
    assert!(first_result.unwrap_err().contains("withdrew vault access"));
    assert!(second_result.unwrap_err().contains("withdrew vault access"));
    assert!(third_result.unwrap_err().contains("withdrew vault access"));
    assert!(
        writer.values.values().is_empty(),
        "the page must receive no password"
    );
    rig.finish().await;
}

#[tokio::test]
async fn revoke_waits_for_an_already_admitted_page_effect() {
    let rig = Rig::new("inflight-admitted", Vec::new()).await;
    rig.saved_always().await;
    let (admitted, mut seen) = watch::channel(false);
    let (release, resume) = watch::channel(false);
    let writer = AdmittedWriter {
        admitted,
        resume,
        values: Writer::default(),
    };
    let fill = tokio::spawn(rig.prepare_with(target(false), writer.clone()).await);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        seen.wait_for(|ready| *ready),
    )
    .await
    .expect("the writer acquired the effect lane")
    .unwrap();
    let shared = rig.core.shared.clone();
    let grant_id = rig.grant_id();
    let mut revoke = tokio::spawn(async move {
        crate::secrets::execute(&shared, crate::SecretsRequest::Revoke { grant_id }).await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke)
            .await
            .is_err(),
        "revoke reported done while an admitted page effect was still pending"
    );
    release.send_replace(true);
    fill.await.unwrap().unwrap();
    revoke.await.unwrap().unwrap();
    assert_eq!(writer.values.values().len(), 1);
    rig.finish().await;
}

/// PREDICT: without an unconfirmed-effect marker, cancelling a caller after dispatch releases its
/// lock while browser work may still run, letting revoke report Done too early.
#[tokio::test]
async fn an_unconfirmed_dispatched_effect_cannot_make_revoke_report_done() {
    let rig = Rig::new("unconfirmed-effect", Vec::new()).await;
    rig.saved_always().await;
    let (admitted, mut seen) = watch::channel(false);
    let (_release, resume) = watch::channel(false);
    let writer = AdmittedWriter {
        admitted,
        resume,
        values: Writer::default(),
    };
    let fill = tokio::spawn(rig.prepare_with(target(false), writer).await);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        seen.wait_for(|ready| *ready),
    )
    .await
    .expect("the page effect was dispatched")
    .unwrap();
    fill.abort();
    assert!(fill.await.unwrap_err().is_cancelled());
    let result = crate::secrets::execute(
        &rig.core.shared,
        crate::SecretsRequest::Revoke {
            grant_id: rig.grant_id(),
        },
    )
    .await;
    assert!(
        result.is_err(),
        "unconfirmed page work cannot yield revoke Done"
    );
    assert!(
        !crate::fill_grants::holds(&rig.core.shared, &rig.login, SITE).await,
        "the persisted grant is withdrawn even when page quiescence is unconfirmed"
    );
    let later = super::FillEffect::new(
        &rig.core.shared,
        &rig.login,
        SITE,
        "grant",
        crate::fill_grants::now_ms(),
    );
    let Err(error) = later.enter().await else {
        panic!("an unconfirmed effect admitted another fill")
    };
    assert!(error.contains("previous browser fill"), "{error}");
    let other_site = "https://other.example.test";
    let other_id = crate::fill_grants::grant_always(
        &rig.core.shared,
        &rig.login,
        "Shop",
        other_site,
        crate::fill_grants::now_ms(),
    )
    .await
    .unwrap();
    let bulk = crate::secrets::execute(
        &rig.core.shared,
        crate::SecretsRequest::RevokeMany {
            grant_ids: vec![rig.grant_id(), other_id],
        },
    )
    .await;
    assert!(
        bulk.is_err(),
        "unconfirmed work cannot make bulk revoke Done"
    );
    assert!(
        !crate::fill_grants::holds(&rig.core.shared, &rig.login, other_site).await,
        "bulk revoke must withdraw the later site despite an earlier unconfirmed effect"
    );
    rig.finish().await;
}

#[tokio::test]
async fn built_in_writer_dispatched_before_revoke_refuses_its_page_effect() {
    let rig = Rig::new_with_preview("built-in-dispatch", Vec::new(), true).await;
    rig.saved_always().await;
    let login = rig.keys.logins().await.unwrap().remove(0);
    let decided = crate::fill_grants::now_ms();
    let (reached, mut at_effect) = watch::channel(false);
    let (resume, release) = watch::channel(false);
    let shared = rig.core.shared.clone();
    let fill = tokio::spawn(async move {
        let driver = crate::browser::browser(&shared).unwrap();
        let writer = super::BrowserLoginWriter {
            driver,
            session_id: CHAT,
            by: vw_browser::Actor::Agent,
            pause_before_effect: Some(Arc::new(super::WriterPause {
                reached,
                resume: release,
            })),
        };
        let field = target(false);
        let effect = super::FillEffect::new(&shared, &login.id, SITE, "grant", decided);
        writer.write(&field, &login, PASSWORD, &effect).await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        at_effect.wait_for(|ready| *ready),
    )
    .await
    .expect("the built-in writer reached its final effect gate")
    .unwrap();
    rig.revoke().await;
    resume.send_replace(true);
    let error = fill.await.unwrap().unwrap_err();
    assert!(error.contains("the user withdrew vault access"), "{error}");
    rig.finish().await;
}

/// The refusal of a revoked fill; neither the page nor the password store was reached.
fn assert_refused(
    result: Result<serde_json::Value, String>,
    writer: &Writer,
    reads: u64,
    reason: &str,
) {
    let error = result.expect_err("a revoked permission must stop the prepared fill");
    assert!(error.contains(reason), "{error}");
    assert!(
        writer.values().is_empty(),
        "the page got a value after the person's change"
    );
    assert_eq!(reads, 0, "the password was read after the person's change");
}

#[tokio::test]
async fn a_saved_always_grant_revoked_before_the_prepared_fill_runs_stops_it() {
    for basic in [false, true] {
        let rig = Rig::new(&format!("saved-{basic}"), Vec::new()).await;
        rig.saved_always().await;
        let writer = Writer::default();
        let prepared = rig.prepare(basic, &writer).await;
        rig.revoke().await;
        let result = prepared.await;
        assert_refused(result, &writer, rig.password_reads(), "revoked");
        // Settings' use log shows the refused fill (no value).
        let last = rig.core.shared.secret_uses.log_entries();
        let last = last.last().expect("a use log line");
        assert_eq!(last.decision, "refused");
        assert_eq!(last.program, format!("browser fill on {SITE}"));
        rig.finish().await;
    }
}

#[tokio::test]
async fn an_always_answer_given_in_the_batch_then_revoked_stops_the_fill() {
    let rig = Rig::new("answered-always", vec![Confirmation::ApprovedAlways]).await;
    let writer = Writer::default();
    let prepared = rig.prepare(false, &writer).await;
    assert!(
        crate::fill_grants::holds(&rig.core.shared, &rig.login, SITE).await,
        "the dialog's Always was saved"
    );
    rig.revoke().await;
    let result = prepared.await;
    assert_refused(result, &writer, rig.password_reads(), "revoked");
    rig.finish().await;
}

#[tokio::test]
async fn an_allow_once_answer_followed_by_a_revoke_of_the_site_stops_the_fill() {
    let rig = Rig::new("answered-once", vec![Confirmation::Approved]).await;
    // An earlier permission for the site that the person already revoked: its record stays, and the dialog asks.
    rig.saved_always().await;
    rig.revoke().await;
    let writer = Writer::default();
    let prepared = rig.prepare(false, &writer).await;
    rig.revoke().await;
    let result = prepared.await;
    assert_refused(result, &writer, rig.password_reads(), "revoked");
    rig.finish().await;
}

#[tokio::test]
async fn an_entry_permission_changed_before_the_prepared_fill_runs_stops_it_before_the_read() {
    let rig = Rig::new("entry-allowed", Vec::new()).await;
    rig.permission(crate::EntryPermissionView::Allowed).await;
    let writer = Writer::default();
    let prepared = rig.prepare(false, &writer).await;
    rig.permission(crate::EntryPermissionView::AskEachTime)
        .await;
    let result = prepared.await;
    assert_refused(
        result,
        &writer,
        rig.password_reads(),
        "entry permission changed",
    );
    rig.finish().await;
}

/// Control: without a revoke the same prepared fills still write the password, once each.
#[tokio::test]
async fn prepared_fills_without_a_revoke_still_fill() {
    let rig = Rig::new("control", vec![Confirmation::Approved]).await;
    let writer = Writer::default();
    // Allow once, then a saved Always for both field kinds.
    rig.prepare(false, &writer).await.await.unwrap();
    rig.saved_always().await;
    for basic in [false, true] {
        rig.prepare(basic, &writer).await.await.unwrap();
    }
    assert_eq!(writer.values(), vec![PASSWORD.to_owned(); 3]);
    assert_eq!(rig.password_reads(), 3);
    rig.finish().await;
}
