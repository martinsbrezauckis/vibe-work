//! A page script on a page without protected fields cannot return a protected value it reached through a frame it
//! made, and a site where a script reached one, or where one was seen, keeps raw actions off afterwards.
//! Real Chromium fixture; the protected page holds synthetic values only.
//!
//! PREDICT (written before the fix): the check before dispatch sees only the clean page, so
//! - (a) a script that frames the same site's checkout page and returns its fields completes with the card number,
//!   PIN and password in its result;
//! - (b) the same script removing the frame before it returns completes with the values too;
//! - (c) a later `1+1` on that site (same page, and another page of the site) completes, so does `storage.read`;
//! - (d) another site is unaffected (passes before and after);
//! - (e) an ordinary script, also one that frames a clean page of the site, returns its value (passes before and
//!   after);
//! - (f) after the check before dispatch refused a raw script on a page with protected fields, a raw script on a
//!   clean page of that site completes;
//! - (g) after a page read withheld a field value, a raw script on a clean page of that site completes.
//!
//! After the fix (a) and (b) fail with the "Result withheld" sentence (the act failed: the script ran), (c) and (f),
//! (g) are refused before dispatch (nothing ran) with the site sentences, and (d), (e) keep working.
//!
//! PREDICT (added with a stricter rule): a check that only matched loaded documents against the frames still present
//! lets (h) through: a script that frames the checkout page, copies its values into the page's own text and empties
//! the frame's fields leaves a frame at the same address with no protected field, so its result completes, and a page
//! read afterwards shows the copied values. With the rule "a document of the page's own site loaded during the run
//! counts as reached" and the reload after a withheld result, (h) is withheld and the read shows no value. (e) then
//! frames another site's page instead of one of its own (a same-site frame loaded by a script counts as reached).
//!
//! PREDICT (added with the rule "any frame that loads a document of the script's own site during the run withholds"
//! and the tab loading its address from before the run again): with the previous rule (i) a script that reads the
//! framed checkout page and then navigates that frame to a clean page of the site completed only because the frame's
//! address no longer matched, so it is withheld already; (j) a script that copies the values into the page's own
//! address (`history.pushState`) leaves them in the address after a reload, so the address still holds them; after
//! the change the tab loads its address from before the run and the copied address is gone.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::protected_origins::{NOT_RUN_ON_SITE, NOT_RUN_SITE_HAS_FIELD, RESULT_WITHHELD};
use vw_browser::{ActError, Actor, Outcome, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

const SYNTHETIC: [&str; 3] = ["5556", "8812", "PWSENT-4417"];

async fn act(rig: &Rig, request: Value) -> Result<Value, ActError> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

async fn script(rig: &Rig, expression: &str) -> Result<Value, ActError> {
    act(rig, json!({"action":"raw.execute","expression":expression})).await
}

/// Frames `src`, waits for it, reads its fields, then runs `after` (`f` the frame, `d` its document, `read` the
/// values), and returns what it read.
fn framing(src: &str, after: &str) -> String {
    format!(
        "(async () => {{ const f = document.createElement('iframe'); f.src = '{src}'; document.body.append(f); \
         await new Promise(r => f.addEventListener('load', r, {{ once: true }})); const d = f.contentDocument; \
         const read = d ? Array.from(d.querySelectorAll('input')).map(i => i.value) : []; {after} \
         return read.join('|') || 'clean'; }})()"
    )
}

async fn open(rig: &Rig, url: &str) {
    rig.service
        .open(vw_preview::OpenRequest {
            session_id: SESSION.into(),
            url: url.parse().unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: true,
            lan: false,
        })
        .await
        .unwrap();
}

fn assert_withheld(result: Result<Value, ActError>, count: u64) {
    let error = match result {
        Ok(value) => panic!("the result left the browser: {value}"),
        Err(error) => error,
    };
    assert_eq!(error.text, RESULT_WITHHELD);
    assert_eq!(error.outcome, Outcome::Failed, "the script ran");
    assert_eq!(error.protected_values_withheld, Some(count));
    for value in SYNTHETIC {
        assert!(!error.text.contains(value));
    }
}

fn assert_refused(result: Result<Value, ActError>, text: &str) {
    let error = match result {
        Ok(value) => panic!("ran on a remembered site: {value}"),
        Err(error) => error,
    };
    assert_eq!(error.text, text);
    assert_eq!(error.outcome, Outcome::Refused, "nothing ran");
    assert_eq!(error.protected_values_withheld, Some(0));
}

#[tokio::test]
async fn a_frame_kept_by_the_script_withholds_its_result_and_the_site_stays_off() {
    let rig = Rig::new().await;
    // (a)
    assert_withheld(script(&rig, &framing("/protected.html", "")).await, 3);
    // (c) the same page (its frame is still there) and another page of the site.
    assert_refused(script(&rig, "1+1").await, NOT_RUN_ON_SITE);
    open(&rig, "https://93.184.215.14/other").await;
    assert_refused(script(&rig, "1+1").await, NOT_RUN_ON_SITE);
    assert_refused(
        act(
            &rig,
            json!({"action":"storage.read","area":"local","keys":["k"]}),
        )
        .await,
        NOT_RUN_ON_SITE,
    );
    // (d) another site.
    open(&rig, "https://other.example.test/").await;
    assert_eq!(
        script(&rig, "1+1").await.unwrap()["untrustedPageContent"]["value"],
        2
    );
}

#[tokio::test]
async fn a_frame_removed_before_returning_withholds_the_result() {
    let rig = Rig::new().await;
    // (b)
    assert_withheld(
        script(&rig, &framing("/protected.html", "f.remove();")).await,
        1,
    );
    open(&rig, "https://93.184.215.14/other").await;
    assert_refused(script(&rig, "1+1").await, NOT_RUN_ON_SITE);
}

#[tokio::test]
async fn ordinary_scripts_and_clean_frames_keep_their_results() {
    let rig = Rig::new().await;
    // (e)
    assert_eq!(
        script(&rig, "1+1").await.unwrap()["untrustedPageContent"]["value"],
        2
    );
    let framed = script(&rig, &framing("https://nested.example.test/child", ""))
        .await
        .unwrap();
    assert_eq!(framed["untrustedPageContent"]["value"], "clean");
    assert_eq!(
        script(&rig, "document.title").await.unwrap()["untrustedPageContent"]["value"],
        "Capture fixture"
    );
}

#[tokio::test]
async fn a_protected_field_seen_before_dispatch_keeps_the_site_off() {
    let rig = Rig::new().await;
    // (f)
    open(&rig, "https://93.184.215.14/protected.html").await;
    let error = script(&rig, "1+1").await.unwrap_err();
    assert_eq!(error.protected_values_withheld, Some(3));
    open(&rig, "https://93.184.215.14/other").await;
    assert_refused(script(&rig, "1+1").await, NOT_RUN_SITE_HAS_FIELD);
    open(&rig, "https://other.example.test/").await;
    assert_eq!(
        script(&rig, "1+1").await.unwrap()["untrustedPageContent"]["value"],
        2
    );
}

#[tokio::test]
async fn a_field_withheld_by_a_page_read_keeps_the_site_off() {
    let rig = Rig::new().await;
    // (g)
    open(&rig, "https://93.184.215.14/protected.html").await;
    let (read, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    for value in SYNTHETIC {
        assert!(!read.to_string().contains(value), "{read}");
    }
    open(&rig, "https://93.184.215.14/other").await;
    assert_refused(script(&rig, "1+1").await, NOT_RUN_SITE_HAS_FIELD);
}

#[tokio::test]
async fn a_frame_emptied_before_returning_withholds_the_result_and_the_page_is_reloaded() {
    let rig = Rig::new().await;
    // (h) copy the values into the page's own text, empty the frame's fields, keep the frame.
    let copy = "document.body.insertAdjacentText('beforeend', 'COPIED ' + read.join(' ')); \
                d.querySelectorAll('input').forEach(i => i.remove());";
    assert_withheld(script(&rig, &framing("/protected.html", copy)).await, 1);
    let (read, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    let text = read.to_string();
    assert!(!text.contains("COPIED"), "{text}");
    for value in SYNTHETIC {
        assert!(!text.contains(value), "{text}");
    }
    assert_refused(script(&rig, "1+1").await, NOT_RUN_ON_SITE);
}

#[tokio::test]
async fn a_frame_navigated_away_before_returning_withholds_the_result() {
    let rig = Rig::new().await;
    // (i) read the checkout page, then send the kept frame to a clean page of the site.
    let away =
        "f.src = '/clean'; await new Promise(r => f.addEventListener('load', r, { once: true }));";
    assert_withheld(script(&rig, &framing("/protected.html", away)).await, 1);
}

/// Opens an address in the session's tab, as the core's navigation does.
struct Reopen<'a>(&'a vw_preview::PreviewService);
impl vw_browser::Navigate for Reopen<'_> {
    fn open<'a>(&'a self, session: &'a str, url: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async move {
            self.0
                .open(vw_preview::OpenRequest {
                    session_id: session.into(),
                    url: url.parse().map_err(|_| "bad address".to_owned())?,
                    allowed_ports: vec![],
                    downloads_dir: None,
                    wait: true,
                    lan: false,
                })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

#[tokio::test]
async fn a_value_copied_into_the_page_address_does_not_stay_there() {
    let rig = Rig::new().await;
    // (j)
    let copy =
        "history.pushState(null, '', '/copied?COPIED-' + encodeURIComponent(read.join('-')));";
    let request = json!({"action":"raw.execute","expression":framing("/protected.html", copy)});
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &Reopen(&rig.service),
        )
        .await;
    assert_withheld(result, 3);
    let address = rig.engine.eval("location.href").await;
    assert_eq!(address, json!(support::URL), "{address}");
}
