//! Page media state and control receipts on real Chromium; the driver executes the production isolated script.
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
async fn act(rig: &Rig, args: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()["untrustedPageContent"]
        .clone()
}
#[tokio::test]
async fn media_reads_controls_and_reports_the_live_state() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<video title=Demo></video><audio title=Sound></audio>';const t=document.querySelector('video').addTextTrack('captions','English','en');t.mode='hidden';t.addCue(new VTTCue(0,3,'A loaded caption.'));").await;
    let read = act(&rig, json!({"action":"media.read"})).await;
    assert_eq!(read["items"].as_array().unwrap().len(), 2);
    let id = &read["items"][0]["mediaId"];
    assert!(id.is_string());
    let muted = act(
        &rig,
        json!({"action":"media.mute","mediaId":id,"muted":true}),
    )
    .await;
    assert_eq!(muted["state"]["muted"], true);
    let paused = act(&rig, json!({"action":"media.pause","mediaId":id})).await;
    assert_eq!(paused["state"]["paused"], true);
    let transcript = act(&rig, json!({"action":"media.transcript","mediaId":id})).await;
    assert_eq!(
        transcript["tracks"][0]["cues"][0]["text"],
        "A loaded caption."
    );
    rig.engine
        .eval("document.querySelector('video').remove()")
        .await;
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"media.pause","mediaId":id})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("media"));
}
#[test]
fn media_rejects_unbounded_or_missing_arguments() {
    assert!(vw_browser::parse(&json!({"action":"media.read"})).is_ok());
    for args in [
        json!({"action":"media.seek","mediaId":"m1","timeMs":-1}),
        json!({"action":"media.mute","mediaId":"m1"}),
        json!({"action":"media.play"}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}

#[tokio::test]
async fn media_text_limits_keep_unicode_receipts_readable() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<video></video>';const v=document.querySelector('video');v.title='a'.repeat(299)+'😀';const t=v.addTextTrack('captions','English','en');t.mode='hidden';t.addCue(new VTTCue(0,3,'b'.repeat(1999)+'😀'));").await;
    let read = act(&rig, json!({"action":"media.read"})).await;
    assert_eq!(read["items"][0]["title"], "a".repeat(299));
    let id = &read["items"][0]["mediaId"];
    let transcript = act(&rig, json!({"action":"media.transcript","mediaId":id})).await;
    assert_eq!(transcript["tracks"][0]["cues"][0]["text"], "b".repeat(1999));
    assert_eq!(transcript["truncated"], true);
}

#[tokio::test]
async fn media_play_and_seek_wait_for_a_real_source_and_report_rejection() {
    let rig = Rig::new().await;
    rig.engine.eval(r"(async()=>{document.body.innerHTML='<audio></audio>';const a=document.querySelector('audio');const bytes=new Uint8Array(44+16000*2),v=new DataView(bytes.buffer);const text=(at,s)=>{for(let i=0;i<s.length;i++)bytes[at+i]=s.charCodeAt(i)};text(0,'RIFF');v.setUint32(4,bytes.length-8,true);text(8,'WAVEfmt ');v.setUint32(16,16,true);v.setUint16(20,1,true);v.setUint16(22,1,true);v.setUint32(24,8000,true);v.setUint32(28,16000,true);v.setUint16(32,2,true);v.setUint16(34,16,true);text(36,'data');v.setUint32(40,bytes.length-44,true);a.src=URL.createObjectURL(new Blob([bytes],{type:'audio/wav'}));await new Promise((resolve,reject)=>{a.onloadedmetadata=resolve;a.onerror=reject;});})()").await;
    let read = act(&rig, json!({"action":"media.read"})).await;
    let id = &read["items"][0]["mediaId"];
    assert_eq!(read["items"][0]["durationMs"], 2000);
    let played = act(&rig, json!({"action":"media.play","mediaId":id})).await;
    assert_eq!(played["state"]["paused"], false);
    act(&rig, json!({"action":"media.pause","mediaId":id})).await;
    let seek = act(
        &rig,
        json!({"action":"media.seek","mediaId":id,"timeMs":1000}),
    )
    .await;
    assert_eq!(seek["state"]["timeMs"], 1000);
    assert_eq!(
        seek["state"]["seeking"], false,
        "a seek receipt must wait for the decoded position"
    );
    rig.engine
        .eval(
            "const a=document.querySelector('audio');a.src='data:audio/wav;base64,AAAA';a.load();",
        )
        .await;
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"media.play","mediaId":id})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("playback failed"));
}

#[tokio::test]
async fn tab_audio_reports_unsupported_engines_without_claiming_element_mute() {
    let rig = Rig::new().await;
    let action = vw_browser::parse(&json!({"action":"tabs.audio"})).unwrap();
    let error = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("media.mute"));
    assert!(vw_browser::parse(&json!({"action":"tabs.mute"})).is_err());
}
