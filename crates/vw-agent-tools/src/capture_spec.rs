//! Optional command-to-vault discovery, with metadata-only arguments and receipts.
//! Depends on the shared vocabulary and wire names; appended by the static MCP tool list.
use serde_json::{Value, json};

pub(crate) fn spec() -> Option<Value> {
    cfg!(feature = "agent-vault").then(|| json!({
        "name": crate::wire::tools::SECRET_CAPTURE_COMMAND,
        "description": format!("Command output can go straight into the vault without entering the chat: name the entry and run one program with arguments. Stdout is private and stderr is discarded; only a clean completion stores the value. {} saves new entries without a card with {} permission; {} and {} ask first and save with {} permission. Duplicates always ask. Input secrets retain their own consent. No shell or inline code: use a script file. Returns only name, source and permission; no output or background job is exposed.",
            vw_vocabulary::label("accessModes", "auto-approve"), vw_vocabulary::label("vaultPermissions", "allowed"),
            vw_vocabulary::label("accessModes", "ask"), vw_vocabulary::label("accessModes", "accept-edits"),
            vw_vocabulary::label("vaultPermissions", "askEachTime")),
        "inputSchema": {"type":"object", "additionalProperties":false, "required":["name","program"],
            "properties": {
                "name":{"type":"string","description":"Vault entry name."},
                "program":{"type":"string","description":"One program on PATH or a path, without arguments."},
                "args":{"type":"array","items":{"type":"string"}},
                "cwd":{"type":"string","description":"Working folder inside this chat's folders."},
                "timeoutSeconds":{"type":"integer","minimum":1,"maximum":30,"default":20,
                    "description":"Private capture finishes within one tool call; timeout stores nothing."},
                "format":{"type":"string","enum":["exact","singleLine"],"default":"exact",
                    "description":"exact preserves UTF-8 text; singleLine removes one final LF/CRLF and rejects other line breaks."},
                "env":{"type":"array","description":"Optional existing secret references for the command's input.",
                    "items":{"type":"object","additionalProperties":false,"required":["name","secret"],
                        "properties":{"name":{"type":"string"},"secret":{"type":"string"},"field":{"type":"string"}}}}
            }}
    }))
}
