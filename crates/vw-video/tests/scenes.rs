//! Owned cut/range regressions through the real worker and its parent validator.
//! Opt-in corpus includes the owned scene-cuts.webm and its independent cut-time manifest.
#![cfg(feature = "helper")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use serde_json::{Value, json};
use vw_video::{
    protocol::{Job, validate},
    worker::extract,
};
fn job(selection: &Value, output: &str) -> Job {
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus plus owned cut fixture");
    Job { id: "cuts".into(), request: serde_json::from_value(json!({
        "path":std::path::Path::new(&root).join(std::env::var("VW_VIDEO_SCENE_FIXTURE").unwrap_or_else(|_| "scene-cuts.webm".into())), "selection":selection,"output":output
    })).unwrap() }
}
fn scenes(start: u64, end: u64, cap: usize) -> Value {
    json!({"kind":"scene_changes","start_ms":start,"end_ms":end,"step_ms":250,"max_frames":cap})
}
#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn scene_cuts_are_deterministic_and_report_full_sampled_coverage() {
    let mut previous = None;
    for _ in 0..2 {
        let j = job(&scenes(0, 4000, 24), "contact_sheet");
        let reply = extract(&j).unwrap();
        validate(&j, &reply).unwrap();
        assert_eq!(
            reply.frames.iter().map(|f| f.actual_ns).collect::<Vec<_>>(),
            [1_000_000_000, 2_000_000_000, 3_000_000_000]
        );
        let value = reply.into_tool();
        if let Some(prior) = &previous {
            assert_eq!(prior, &value);
        }
        previous = Some(value.clone());
        assert_eq!(value["scan"]["start_ms"], 0);
        assert_eq!(value["scan"]["end_ms"], 4000);
        assert_eq!(value["scan"]["step_ms"], 250);
        assert_eq!(value["scan"]["samples"], 16);
        assert_eq!(value["scan"]["last_sample_ms"], 3750);
        assert_eq!(value["scan"]["detected_changes"], 3);
        assert_eq!(value["scan"]["omitted_changes"], 0);
        assert!(value["coverage"].as_str().unwrap().contains("Request"));
    }
}
#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn scene_budget_range_and_no_cut_results_are_honest() {
    for (start, end, cap, expected, omitted) in [
        (0, 4000, 2, vec![1000, 2000], 1),
        (1250, 3250, 24, vec![2000, 3000], 0),
        (0, 750, 24, vec![], 0),
    ] {
        let j = job(&scenes(start, end, cap), "contact_sheet");
        let reply = extract(&j).unwrap();
        validate(&j, &reply).unwrap();
        assert_eq!(
            reply
                .frames
                .iter()
                .map(|f| f.requested_ms)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(reply.image_count, usize::from(!expected.is_empty()));
        assert_eq!(reply.into_tool()["scan"]["omitted_changes"], omitted);
    }
}
#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn range_end_outside_video_is_refused_even_when_samples_are_inside() {
    for selection in [
        json!({"kind":"uniform","start_ms":0,"end_ms":4500,"count":1}),
        json!({"kind":"interval","start_ms":0,"end_ms":4500,"every_ms":5000,"max_frames":24}),
    ] {
        for output in ["contact_sheet", "metadata"] {
            let error = extract(&job(&selection, output)).unwrap_err();
            assert!(error.contains("range_outside_video"), "{error}");
            assert!(error.contains("4000"), "{error}");
            assert!(error.contains("Choose"), "{error}");
        }
    }
}
#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn ranges_select_only_requested_moments_and_allow_end_at_duration() {
    for (selection, expected) in [
        (
            json!({"kind":"uniform","start_ms":1000,"end_ms":3000,"count":2}),
            vec![1500, 2500],
        ),
        (
            json!({"kind":"interval","start_ms":1000,"end_ms":4000,"every_ms":1000,"max_frames":3}),
            vec![1000, 2000, 3000],
        ),
    ] {
        let j = job(&selection, "contact_sheet");
        let reply = extract(&j).unwrap();
        validate(&j, &reply).unwrap();
        assert_eq!(
            reply
                .frames
                .iter()
                .map(|f| f.requested_ms)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn scene_metadata_and_single_frame_preserve_counts_and_reject_forged_coverage() {
    for output in ["metadata", "frame"] {
        let j = job(&scenes(0, 4000, 1), output);
        let reply = extract(&j).unwrap();
        validate(&j, &reply).unwrap();
        assert_eq!(reply.frames.len(), 1);
        assert_eq!(reply.image_count, usize::from(output == "frame"));
        let value = serde_json::to_value(reply).unwrap();
        for (field, bad) in [
            ("samples", json!(1)),
            ("step_ms", json!(500)),
            ("last_sample_ms", json!(4000)),
            ("detected_changes", json!(24)),
            ("omitted_changes", json!(0)),
        ] {
            let mut forged = value.clone();
            forged["scan"][field] = bad;
            let reply = serde_json::from_value(forged).unwrap();
            assert!(validate(&j, &reply).is_err(), "{field}");
        }
        for bad in [0, 999, 4000] {
            let mut forged = value.clone();
            forged["frames"][0]["requested_ms"] = json!(bad);
            assert!(validate(&j, &serde_json::from_value(forged).unwrap()).is_err());
        }
    }
}
#[test]
#[ignore = "requires the owned cut fixture and installed system decoder"]
fn scene_ranges_outside_video_and_unsampled_cuts_are_explicit() {
    for (start, end) in [(0, 4500), (4000, 4500), (5000, 6000)] {
        let error = extract(&job(&scenes(start, end, 24), "contact_sheet")).unwrap_err();
        assert!(error.contains("range_outside_video"), "{error}");
    }
    let mut selection = scenes(0, 4000, 24);
    selection["step_ms"] = json!(1500);
    let j = job(&selection, "contact_sheet");
    let reply = extract(&j).unwrap();
    validate(&j, &reply).unwrap();
    assert_eq!(
        reply
            .frames
            .iter()
            .map(|f| f.requested_ms)
            .collect::<Vec<_>>(),
        [1500, 3000]
    );
    assert_eq!(reply.scan.as_ref().unwrap().samples, 3);
    assert_eq!(reply.scan.as_ref().unwrap().last_sample_ms, 3000);
}
