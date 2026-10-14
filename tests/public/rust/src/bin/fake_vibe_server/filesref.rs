//! The fake's file references (the fixture): a turn whose text contains `filesref-fixture` first records an assistant
//! message that mentions files in every shape the File tab must handle (backticks, plain, a Markdown link, a path
//! with a line, absolute paths outside the folder, a traversal, a symlink out, words that are not paths), and a
//! completed `file_read` effect of `docs/plan.md` (the step names its file). The e2e creates the files.

use serde_json::{Value, json};

use crate::script::Script;

/// The message text (Markdown).
pub const MESSAGE: &str = "I looked at `src/main.rs` and docs/plan.md:3, and wrote [the report](site/page.html). \
Also see site/doc.pdf, logo.png, big.log and notes.md. Not files: /etc/passwd, ../outside.txt, link-out.txt, \
`npm run build`, e.g. and https://example.com/a.md. See /tmp/a and then read notes.md.";

impl Script {
    /// Records the fixture message and step when the turn's text asks for them.
    pub(crate) fn maybe_files_refs(&self, sid: &str, turn_id: &str, params: &Value) {
        if !params["message"].to_string().contains("filesref-fixture") {
            return;
        }
        let read = self.next("effect-read");
        self.add_entry(
            sid,
            json!({"id": read, "type": "effect", "turnId": turn_id, "sessionId": sid,
                "createdAt": 2, "updatedAt": 3, "generationStatus": "completed", "relatedEntryId": null,
                "title": "read_file",
                "detail": {"toolName": "read_file", "kind": "file_read", "input": {"filePath": "docs/plan.md"},
                    "display": {"summary": "Read docs/plan.md", "verb": "Reading", "message": "docs/plan.md",
                        "settledVerb": "Read", "settledMessage": "docs/plan.md", "statusText": "Reading"}},
                "state": {"status": "completed", "output": {"content": "# Plan\n"}, "outputText": "",
                    "durationMs": 12.0, "display": {"success": true, "message": "docs/plan.md"},
                    "decision": null, "approvalType": null, "approvalSource": null}}),
        );
        let message = self.next("assistant-files");
        self.add_entry(
            sid,
            json!({"id": message, "type": "message", "role": "assistant", "turnId": turn_id,
                "content": [{"type": "text", "text": MESSAGE}], "generationStatus": "completed"}),
        );
    }
}
