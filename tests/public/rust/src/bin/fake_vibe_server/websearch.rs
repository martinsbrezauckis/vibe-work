//! Synthetic web-search effect for link rendering tests. A turn containing `websearch-fixture`
//! adds a completed connector result with ordinary and suspicious sample URLs.

use serde_json::{Value, json};

use crate::script::Script;

/// The sources the fixture search returns.
pub fn fixture_sources() -> Value {
    json!([
        {"title": "Harbour history - City archive", "url": "https://archive.example.org/harbour/1887?ref=vw"},
        {"title": "bank.example (looks like a bank)", "url": "https://bank.example@evil.example/login"},
        {"title": "Müller Hafen", "url": "https://müller.example/hafen"},
        {"title": "<img src=x onerror=alert(1)>", "url": "javascript:alert(1)"}
    ])
}

impl Script {
    /// Records the fixture search when the turn's text asks for it.
    pub(crate) fn maybe_web_search(&self, sid: &str, turn_id: &str, params: &Value) {
        if !params["message"].to_string().contains("websearch-fixture") {
            return;
        }
        let id = self.next("effect-websearch");
        let query = "harbour opening year";
        let tool = "connector_web_search.web_search";
        let mut refs = serde_json::Map::new();
        for (rank, source) in fixture_sources()
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            refs.insert(
                format!("ref{rank}"),
                json!({"title": source["title"], "url": source["url"], "rank": rank,
                    "description": null, "snippets": ["(fixture)"], "can_open": true,
                    "metadata": {"favicon": "https://favicon.example/x.png"}}),
            );
        }
        let refs = Value::Object(refs);
        let text = refs.to_string();
        self.add_entry(
            sid,
            json!({"id": id, "type": "effect", "turnId": turn_id, "sessionId": sid,
                "createdAt": 2, "updatedAt": 3, "generationStatus": "completed", "relatedEntryId": null,
                "title": tool,
                "detail": {"toolName": tool, "kind": "tool", "input": {"query": query},
                    "display": {"summary": tool, "verb": "Running", "message": tool,
                        "settledVerb": "Ran", "settledMessage": tool,
                        "statusText": format!("Running {tool}")}},
                "state": {"status": "completed",
                    "output": {"type": "success", "content": [{"type": "text", "text": text}],
                        "structured_content": refs, "_meta": {}},
                    "outputText": text, "durationMs": 812.0,
                    "display": {"success": true, "message": tool},
                    "decision": null, "approvalType": null, "approvalSource": null}}),
        );
    }
}
