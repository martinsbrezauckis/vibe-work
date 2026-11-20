//! The Local Network Access question ([`super::question`]) and its bounded wait ([`super::ASK_WAIT`]).
//!
//! PREDICT (written before the change): the question never said whether the page asked over HTTPS, so after Allow a
//! plain-HTTP local server still failed and the person read it as "Allow is broken". An HTTPS request (a tunnel)
//! now carries one line that says so; a plain request does not.

use std::sync::Arc;
use vw_netguard::LocalRequest;
use vw_netguard::ip::IpClass;
use vw_platform::sync::LockRecover as _;
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, Placement, PreviewEngine,
    PreviewHooks, Storage,
};

struct NoPageCalls;
impl PreviewEngine for NoPageCalls {
    fn attach(&self, _: Arc<dyn PreviewHooks>) {}
    fn mount(&self, _: MountSpec) -> EngineFuture<'_, ()> {
        panic!("no page may mount")
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        panic!("no page may navigate")
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        panic!("no history")
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        panic!("no page may move")
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        panic!("no page may close")
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        panic!("no screenshot")
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        panic!("no console")
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<std::path::PathBuf>> {
        panic!("no storage")
    }
}

fn request(tls: bool) -> LocalRequest<'static> {
    LocalRequest {
        class: IpClass::Loopback,
        port: 8080,
        host: "127.0.0.1",
        own_address: false,
        initiator: Some("https://public.example"),
        nested: false,
        tls,
    }
}

#[tokio::test]
async fn an_https_request_says_that_a_plain_http_server_will_still_not_answer() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let shared = &fixture.core.shared;
    let line = vw_vocabulary::texts::LOCAL_NETWORK_HTTPS;
    assert!(
        line.contains("HTTPS") && line.contains("after Allow"),
        "{line}"
    );
    let https = super::question(shared, "https://public.example", "chat", &request(true));
    assert!(https.summary.contains(line), "{}", https.summary);
    assert!(
        https.summary.find(line) < https.summary.find("Allow it only if you trust"),
        "said before the answer: {}",
        https.summary
    );
    let plain = super::question(shared, "https://public.example", "chat", &request(false));
    assert!(!plain.summary.contains("HTTPS"), "{}", plain.summary);
    fixture.finish().await;
}

/// A person who never answers; counts the prompts.
struct Silent(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl vw_gate::Confirmer for Silent {
    async fn confirm(&self, _: &vw_gate::ConfirmationRequest) -> vw_gate::Confirmation {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::pending().await
    }
}

/// PREDICT: no test bounded the wait; a question nobody answers must refuse its requests after [`super::ASK_WAIT`]
/// (not hold them forever), and a second request of the same site joins the one open question (one prompt).
#[tokio::test]
async fn an_unanswered_question_refuses_its_requests_after_the_bounded_wait() {
    let prompts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let core = crate::offline_core::start_with_confirmer(
        "local-network-unanswered-timeout",
        |config, root| {
            config.preview = Some(crate::PreviewSetup {
                engine: Arc::new(NoPageCalls),
                config: vw_preview::PreviewConfig::new(root.join("preview")),
                guard: None,
            });
        },
        Silent(prompts.clone()),
    )
    .await;
    let shared = &core.shared;
    let mut events = core.subscribe(None);
    let context = super::GuardContext::new(shared);
    // Started on the real clock; the wait itself runs on paused time, which jumps when every task waits.
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let site = "https://public.example";
    let (one, two) = (request(false), request(false));
    let (first, second) = tokio::join!(
        context.ask(shared, site, "chat", &one),
        context.ask(shared, site, "chat", &two),
    );
    let waited = started.elapsed();
    assert_eq!((first, second), (None, None), "no answer refuses");
    assert_eq!(
        prompts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one question per site"
    );
    assert!(
        waited >= super::ASK_WAIT && waited <= super::ASK_WAIT + std::time::Duration::from_secs(1),
        "{waited:?}"
    );
    assert!(
        context.pending.lock_or_recover().is_empty(),
        "the question is closed"
    );
    // PREDICT: the bounded Ask path returns before recording an unanswered site or emitting a person-visible alert.
    let listed = crate::browser_guard::sites(
        shared,
        &crate::browser_view::BrowserRequest::LocalNetworkSites {},
        false,
    )
    .await
    .expect("site list");
    assert!(
        format!("{listed:?}").contains(site),
        "the unanswered site must be listed: {listed:?}"
    );
    tokio::time::resume();
    let mut alert = false;
    for _ in 0..16 {
        let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.next())
            .await
            .expect("alert arrives")
        else {
            break;
        };
        if let crate::event::CoreEvent::Alert {
            session_id,
            kind,
            message,
        } = event
            && kind == crate::event::AlertKind::LocalNetworkUnanswered
        {
            assert_eq!(session_id.as_deref(), Some("chat"));
            assert_eq!(
                message,
                format!(
                    "A request from {site} to your local network was refused because the question was not answered."
                )
            );
            alert = true;
            break;
        }
    }
    assert!(alert, "the timeout must reach the person's bell and notice");
    core.shutdown().await;
}
