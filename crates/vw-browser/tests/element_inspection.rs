//! One-element inspection on the real Chromium fixture: computed CSS, literal attributes and bounded safe markup.
//! The production driver supplies ownership and vault redaction; no arbitrary page expression is accepted.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::Actor;

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn inspect(rig: &Rig, args: Value) -> Result<Value, String> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args)?,
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|e| e.text)
}

#[tokio::test]
async fn inspected_element_returns_styles_and_raw_attribute_without_password_or_script_content() {
    let rig = Rig::new().await;
    rig.engine.eval(r#"document.body.innerHTML = '<article id="sample" title="a &amp; b" style="color:rgb(12, 34, 56)"><b>Hello</b><input type="password" value="private-password"><script>private-script-token</script></article>'"#).await;
    let answer = inspect(&rig, json!({"action":"page.inspect","selector":"#sample","attribute":"title","properties":["color"]})).await.unwrap();
    let data = &answer["untrustedPageContent"];
    assert_eq!(data["attribute"]["value"], "a & b");
    assert_eq!(data["styles"]["color"], "rgb(12, 34, 56)");
    assert!(data["html"].as_str().unwrap().contains("<b>Hello</b>"));
    assert!(!answer.to_string().contains("private-password"));
    assert!(!answer.to_string().contains("private-script-token"));
    assert_eq!(answer["ok"], true);
}

#[tokio::test]
async fn missing_ambiguous_or_secret_targets_do_not_claim_inspection() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<button>A</button><button>B</button><input type=password value=secret>'").await;
    for selector in ["button", "#missing", "[", "input"] {
        assert!(
            inspect(
                &rig,
                json!({"action":"page.inspect","selector":selector,"attribute":"value"})
            )
            .await
            .is_err(),
            "{selector}"
        );
    }
}

#[test]
fn inspection_bounds_are_checked_before_evaluation() {
    assert!(vw_browser::parse(&json!({"action":"page.inspect","selector":"#one"})).is_ok());
    for args in [
        json!({"action":"page.inspect"}),
        json!({"action":"page.inspect","selector":"x".repeat(1001)}),
        json!({"action":"page.inspect","selector":"#one","properties":["color;fetch()"]}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}
