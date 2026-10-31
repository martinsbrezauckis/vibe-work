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

fn hook(id: &str, kind: &str, message: &str, detail: Value) -> Value {
    let mut detail = detail;
    detail["kind"] = json!(kind);
    entry(
        id,
        "notice",
        json!({"level": "info", "message": message, "detail": detail}),
    )
}

/// Hook notices follow the window's rule (`ui/src/features/transcript/model/hookNotices.ts`): lifecycle and no-op
/// notices are left out, an outcome is written as the window words it, and a call a hook refused is written once.
#[test]
fn hook_notices_follow_the_windows_rule() {
    let head = ExportHead {
        cwd: Some("/work/calc".into()),
        session_id: "hooks".into(),
        ..ExportHead::default()
    };
    let mut skipped = effect("e9", "shell", "Run", "rm -rf build", "skipped", Value::Null);
    skipped["state"]["reason"] =
        json!("<tool_error>Tool 'bash' was denied by hook 'guard': builds are kept</tool_error>");
    skipped["detail"]["toolName"] = json!("bash");
    let entries = [
        entry(
            "u1",
            "message",
            json!({"role": "user", "content": text("go")}),
        ),
        hook("h1", "hook_run_started", "Running hooks", json!({})),
        hook(
            "h2",
            "hook_completed",
            "Hook fmt completed",
            json!({"hookName": "fmt", "status": "ok", "content": "", "scope": "post_tool"}),
        ),
        hook(
            "h3",
            "hook_completed",
            "Hook lint completed",
            json!({"hookName": "lint", "status": "ok", "content": "2 files fixed", "scope": "post_tool"}),
        ),
        skipped,
        hook(
            "h4",
            "hook_completed",
            "Hook guard completed",
            json!({"hookName": "guard", "status": "error", "content": "Denied tool 'bash'",
                "scope": "pre_tool", "toolCallId": "e9"}),
        ),
        hook("h5", "hook_run_completed", "Hooks completed", json!({})),
        entry(
            "n1",
            "notice",
            json!({"level": "info", "message": "Plan approved"}),
        ),
    ];
    let (md, _) = render(&head, &entries);
    assert!(!md.contains("Running hooks"), "{md}");
    assert!(!md.contains("Hooks completed"), "{md}");
    assert!(!md.contains("fmt"), "a no-op hook is left out: {md}");
    assert!(md.contains("> lint: 2 files fixed"), "{md}");
    assert!(md.contains("> builds are kept"), "{md}");
    assert!(
        !md.contains("rm -rf build"),
        "the refused call is written once, as the hook's reason: {md}"
    );
    assert!(md.contains("> Plan approved"), "{md}");
}

/// Diff headers name a file in the chat's folder relative to it; any other file by its own path, never `a//`.
#[test]
fn diff_paths_are_relative_to_the_chats_folder() {
    let head = ExportHead {
        cwd: Some("/work/calc".into()),
        session_id: "paths".into(),
        ..ExportHead::default()
    };
    let write = |id: &str, path: &str| {
        effect(
            id,
            "file_write",
            "Write",
            path,
            "completed",
            json!({"filePath": path, "content": "x\n"}),
        )
    };
    let entries = [
        write("e1", "/work/calc/src/a.py"),
        write("e2", "/etc/vibe.conf"),
        write("e3", "notes.md"),
        write("e4", "/work/calculator/b.py"),
    ];
    let (md, counts) = render(&head, &entries);
    assert_eq!(counts.diffs, 4);
    assert!(md.contains("--- a/src/a.py\n+++ b/src/a.py\n"), "{md}");
    assert!(
        md.contains("--- /etc/vibe.conf\n+++ /etc/vibe.conf\n"),
        "{md}"
    );
    assert!(md.contains("--- a/notes.md\n+++ b/notes.md\n"), "{md}");
    assert!(
        md.contains("--- /work/calculator/b.py\n"),
        "a sibling folder is not inside: {md}"
    );
    assert!(!md.contains("a//"), "{md}");
}

/// The app's own context block (`vw://app/context`) is not a person's attachment; their context items stay noted.
#[test]
fn the_apps_own_context_is_not_noted_as_an_attachment() {
    let head = ExportHead {
        session_id: "context".into(),
        ..ExportHead::default()
    };
    let entries = [entry(
        "u1",
        "message",
        json!({"role": "user", "content": [
            {"type": "text", "text": "fix it"},
            {"type": "resource", "resource": {"kind": "text", "uri": "vw://app/context", "text": "app facts"}},
            {"type": "resource", "resource": {"kind": "text", "uri": "vw://context/1", "name": "Selection", "text": "x"}}
        ]}),
    )];
    let (md, _) = render(&head, &entries);
    assert!(!md.contains("vw://app/context"), "{md}");
    assert!(md.contains("*[attached: Selection]*"), "{md}");
}
