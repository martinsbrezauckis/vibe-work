//! Markdown formatting of supplied public entry data, independent of Vibe or any server fixture.
//! Uses `export_md` directly; real-server integration tests separately verify paging, saving and core secret scrubbing.
use crate::export_md::{ExportHead, render};
use serde_json::{Value, json};
const SECRET: &str = "sk-se1-export-secret-9f8e7d6c5b4a";

fn entry(id: &str, kind: &str, extra: Value) -> Value {
    let mut base =
        json!({"id": id, "type": kind, "createdAt": 1_791_447_549_000_u64, "turnId": "t1"});
    let Value::Object(extra) = extra else {
        panic!("entry fields must be an object")
    };
    base.as_object_mut().unwrap().extend(extra);
    base
}

fn text(t: &str) -> Value {
    json!([{"type": "text", "text": t}])
}

fn effect(id: &str, kind: &str, verb: &str, message: &str, status: &str, output: Value) -> Value {
    let settled = match verb {
        "Run" => "Ran".to_owned(),
        "Write" => "Wrote".to_owned(),
        other => format!("{other}ed"),
    };
    let mut value = entry(
        id,
        "effect",
        json!({"detail": {"kind": kind, "display": {"verb": verb, "message": message,
                  "settledVerb": settled, "settledMessage": message}, "input": {"filePath": message}},
               "state": {"status": status,
                  "display": {"verb": "", "message": "", "success": status == "completed"}}}),
    );
    value["state"]["output"] = output;
    value
}

fn rich_history() -> Value {
    json!([
        entry(
            "u1",
            "message",
            json!({"role": "user", "content": text("Add a multiply helper to calc.py")})
        ),
        entry(
            "r1",
            "reasoning",
            json!({"content": text("SECRET-REASONING-TEXT")})
        ),
        effect(
            "e1",
            "shell",
            "Run",
            "python -m pytest -q",
            "failed",
            Value::Null
        ),
        effect(
            "e2",
            "file_edit",
            "Edit",
            "calc.py",
            "completed",
            json!({"file": "calc.py", "occurrences": [
            {"startLine": 1, "oldText": "def add(a, b):\n    return a + b",
             "newText": "def add(a, b):\n    return a + b\n\n\ndef multiply(a, b):\n    return a * b\n# ```"}]})
        ),
        effect(
            "e3",
            "file_write",
            "Write",
            "notes.md",
            "completed",
            json!({"filePath": "notes.md", "content": "hello\n", "fileExisted": false})
        ),
        entry(
            "a1",
            "message",
            json!({"role": "assistant",
            "content": text(&format!("Done. The key `{SECRET}` was in the env.\n\n```python\nprint(1)\n```"))})
        ),
        entry(
            "n1",
            "notice",
            json!({"level": "info", "message": "Plan approved"})
        ),
    ])
}

#[test]
fn rich_entries_preserve_steps_diffs_fences_and_omit_reasoning() {
    let head = ExportHead {
        title: Some("Add multiply".into()),
        cwd: Some("/work/calc".into()),
        model: Some("large-4".into()),
        session_id: "format-only".into(),
        exported_at_ms: 1_791_447_549_000,
    };
    let (md, counts) = render(&head, rich_history().as_array().unwrap());
    assert!(md.starts_with("# Add multiply\n"), "{md}");
    assert!(md.contains("- Folder: `/work/calc`"));
    assert!(md.contains("- Model: `large-4`"));
    let you = md.find("## You").unwrap();
    let vibe = md.find("## Vibe").unwrap();
    assert!(you < vibe);
    assert!(md.contains("Add a multiply helper to calc.py"));
    assert!(
        md.contains("- Ran `python -m pytest -q` (failed)"),
        "failed step marked: {md}"
    );
    assert!(md.contains("- Edited `calc.py`"), "{md}");
    assert!(md.contains("+def multiply(a, b):"), "edit diff: {md}");
    assert!(
        md.contains("````diff"),
        "a fence longer than the ``` inside the edit: {md}"
    );
    assert!(md.contains("+hello"), "new file as added lines");
    assert!(!md.contains("SECRET-REASONING-TEXT"), "reasoning left out");
    assert!(md.contains("> Plan approved"));
    assert_eq!((counts.messages, counts.steps, counts.diffs), (2, 3, 2));
}
