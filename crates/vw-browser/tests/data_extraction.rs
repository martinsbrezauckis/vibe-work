//! Structured extraction through the production browser/frame reader on a disposable DOM.
//! Verifies merged frame batches and read-only behavior before the core saves an artifact.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::ReadOptions;

#[tokio::test]
async fn table_extraction_preserves_headers_rows_and_frame_sources() {
    let rig = Rig::new().await;
    rig.engine.eval(r"document.body.innerHTML='<table><tr><th>Item</th><th>Count</th></tr><tr><td>Top</td><td>2</td></tr></table><iframe></iframe>';const doc=document.querySelector('iframe').contentDocument;doc.body.innerHTML='<table><tr><th>Item</th><th>Count</th></tr><tr><td>Child</td><td>3</td></tr></table>';").await;
    let options = ReadOptions {
        extract: Some(serde_json::from_value(json!({"kind":"table"})).unwrap()),
        ..Default::default()
    };
    let (model, _, _) = rig.browser.observe(SESSION, options).await.unwrap();
    assert_eq!(model.datasets.len(), 2);
    assert_eq!(model.datasets[0].rows, vec![vec!["Top", "2"]]);
    assert_eq!(model.datasets[1].rows, vec![vec!["Child", "3"]]);
    assert!(
        model
            .datasets
            .iter()
            .all(|data| data.columns == ["Item", "Count"] && !data.truncated)
    );
}
