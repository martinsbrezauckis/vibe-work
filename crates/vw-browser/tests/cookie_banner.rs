//! A Cookiebot region's hidden vendor tables cannot starve its visible footer controls.
//! Runs the production read/action pipeline through the offline Chromium fixture.
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

async fn banner() -> Rig {
    let rig = Rig::new().await;
    let html = serde_json::to_string(include_str!("support/cookiebot.html")).unwrap();
    rig.engine
        .eval(&format!(
            r"(() => {{
              const html = new DOMParser().parseFromString({html}, 'text/html');
              document.head.innerHTML = html.head.innerHTML;
              document.body.innerHTML = html.body.innerHTML;
              document.body.style = 'margin:0';
              const rows = '<div><span>Local vendor</span></div>'.repeat(100);
              document.querySelector('#cookie-details').innerHTML = ('<section>' + rows + '</section>').repeat(65);
              document.querySelectorAll('button').forEach(b => b.addEventListener('click', () => {{
                document.body.dataset.clicked = b.textContent;
                document.body.dataset.clicks = Number(document.body.dataset.clicks || 0) + 1;
              }}));
              scrollTo(0, 0);
            }})()"
        ))
        .await;
    rig
}

#[tokio::test]
async fn cookiebot_hidden_tables_do_not_starve_visible_footer_refs_or_clicks() {
    let rig = banner().await;
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
        ["Noraidīt", "Pielāgot", "Atļaut visu"]
    );
    for name in ["Noraidīt", "Pielāgot", "Atļaut visu"] {
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
        let refs = answer["untrustedPageContent"]["refs"].as_array().unwrap();
        let reference = refs
            .iter()
            .find(|r| r["name"] == name && r["visible"] != false)
            .expect("visible Cookiebot footer button")["ref"]
            .as_str()
            .unwrap();
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
    assert_eq!(rig.engine.eval("document.body.dataset.clicks").await, "3");
}

#[tokio::test]
async fn focused_read_without_a_usable_ref_gives_a_different_next_step() {
    let rig = banner().await;
    for query in ["Allow all", "Noraidīt visu"] {
        let (answer, _) = rig
            .browser
            .read(
                SESSION,
                ReadOptions {
                    find: Some(query.into()),
                    ..ReadOptions::default()
                },
                false,
                Actor::Agent,
            )
            .await
            .unwrap();
        assert!(
            answer["nextStep"]
                .as_str()
                .is_some_and(|s| s.contains("without find")),
            "{answer}"
        );
        assert!(
            answer["nextStep"]
                .as_str()
                .unwrap()
                .contains("Do not repeat")
        );
        assert!(vw_browser::observe::size(&answer) <= vw_browser::observe::READ_BUDGET);
    }
}

#[tokio::test]
async fn banner_in_open_shadow_root_of_a_zero_size_host_is_still_clickable() {
    let rig = banner().await;
    rig.engine
        .eval(
            r"(() => {
      const host = document.createElement('div');
      host.style = 'width:0;height:0';
      document.body.prepend(host);
      const shadow = host.attachShadow({mode:'open'});
      shadow.append(document.querySelector('style').cloneNode(true));
      shadow.append(document.querySelector('#CybotCookiebotDialog'));
    })()",
        )
        .await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Atļaut visu".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let button = model.refs.first().expect("shadow banner button");
    assert_eq!(button.name, "Atļaut visu");
    assert_ne!(button.visible, Some(false));
    rig.browser
        .act(
            SESSION,
            Action::Click {
                reference: button.id.clone(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("document.body.dataset.clicked").await,
        "Atļaut visu"
    );
}

#[tokio::test]
async fn prioritizing_a_region_never_makes_its_hidden_controls_clickable() {
    let rig = banner().await;
    rig.engine.eval("document.querySelector('#cookie-details').replaceChildren(); document.querySelector('#CybotCookiebotDialog').style.opacity='0'").await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Atļaut visu".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let button = model.refs.first().expect("hidden banner button");
    assert_eq!(button.visible, Some(false));
    let error = rig
        .browser
        .act(
            SESSION,
            Action::Click {
                reference: button.id.clone(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(error.text.contains("not visible"), "{error:?}");
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn same_origin_frame_banner_gets_refs_and_hit_testing() {
    let rig = banner().await;
    rig.engine
        .eval(
            r"(() => {
      const frame = document.createElement('iframe');
      frame.style = 'position:fixed;inset:0;width:300px;height:200px;border:0';
      document.body.prepend(frame);
      frame.contentDocument.head.append(document.querySelector('style').cloneNode(true));
      frame.contentDocument.body.append(document.querySelector('#CybotCookiebotDialog'));
    })()",
        )
        .await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Noraidīt".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let button = model
        .refs
        .iter()
        .find(|r| r.name == "Noraidīt")
        .expect("framed banner button");
    assert_eq!(button.frame, Some(true));
    assert_ne!(button.visible, Some(false));
    rig.browser
        .act(
            SESSION,
            Action::Click {
                reference: button.id.clone(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("document.body.dataset.clicked").await,
        "Noraidīt"
    );
}
