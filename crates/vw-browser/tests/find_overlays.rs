//! Browser finding: focused refs survive large pages and visible overlays precede the body.
//! Uses the production driver with a real Chromium fixture, independent of any agent session.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use support::{Rig, SESSION};
use vw_browser::{Action, Actor, NavFuture, Navigate, ReadOptions};

struct NoNavigation;
impl Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test]
async fn banner_controls_and_focused_body_refs_survive_fifteen_hundred_links() {
    let rig = Rig::new().await;
    rig.engine.eval(r##"(() => {
        document.head.innerHTML = '<title>Many links</title>';
        document.body.style = 'margin:0';
        document.body.innerHTML = Array.from({length:1500}, (_, i) =>
          `<a href="#link-${i}">Link ${i}</a><br>`).join('') +
          '<div role="dialog" aria-modal="true" style="position:fixed;inset:0 0 auto;background:white;z-index:100">' +
          ['Deny','Customize','Allow all'].map(name =>
            `<button onclick="document.body.dataset.clicked=this.textContent">${name}</button>`).join('') + '</div>';
        scrollTo(0, 0);
    })()"##).await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                max_refs: 3,
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        model
            .refs
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["Deny", "Customize", "Allow all"]
    );
    for name in ["Deny", "Customize", "Allow all"] {
        let (answer, _) = rig
            .browser
            .read(
                SESSION,
                ReadOptions {
                    find: Some(name.into()),
                    ..ReadOptions::default()
                },
                false,
                Actor::Agent,
            )
            .await
            .unwrap();
        let reference = answer["untrustedPageContent"]["refs"][0]["ref"]
            .as_str()
            .expect("focused banner ref");
        rig.browser
            .act(
                SESSION,
                Action::Click {
                    reference: reference.into(),
                },
                Actor::Agent,
                &NoNavigation,
            )
            .await
            .unwrap();
        assert_eq!(rig.engine.eval("document.body.dataset.clicked").await, name);
    }
    // Finding a body item must work independently of overlay prioritization and the 1,000-ref cache.
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Link 1499".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let found = model
        .refs
        .iter()
        .find(|r| r.name == "Link 1499")
        .expect("late body ref");
    assert!(model.all_refs.contains(&found.id));
    // Fixed top layers without ARIA dialog metadata get the same ordering.
    rig.engine.eval("document.querySelector('[role=dialog]').removeAttribute('role'); document.querySelector('[aria-modal]').removeAttribute('aria-modal')").await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                max_refs: 3,
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(model.refs[0].name, "Deny");
}
