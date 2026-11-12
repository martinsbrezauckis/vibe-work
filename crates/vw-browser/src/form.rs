//! Bounded multi-field form entry using the ordinary checked action driver and its live postconditions.
//! Agent tools call this after shared-tier consent; parsing and initial ref checks finish before any input.
use crate::{Actor, Browser, Navigate, act::Action, observe, parse};
use serde_json::{Value, json};

/// Validates a batch of one to sixteen type/select/check field operations, without submission.
///
/// # Errors
/// Malformed, oversized or unsupported fields refuse before any operation runs.
pub fn parse_form(args: &Value) -> Result<Vec<Action>, String> {
    let fields = args["fields"].as_array().filter(|fields| (1..=16).contains(&fields.len()))
        .ok_or("A form needs 1 to 16 fields; give each an action (type, select or check) and an observed ref.")?;
    let mut length = 0;
    fields.iter().map(|field| {
        let object = field.as_object().ok_or("Each form field must be an object with an action and ref.")?;
        if object.keys().any(|key| !["action", "ref", "text", "value", "values", "checked"].contains(&key.as_str())) {
            return Err("Form fields accept only action, ref, text, value, values and checked; submission is a separate action.".into());
        }
        if !matches!(field["action"].as_str(), Some("type" | "select" | "check" | "uncheck")) {
            return Err("A form field supports type, select, check or uncheck; use other actions separately.".into());
        }
        length += field["text"].as_str().map_or(0, str::len);
        if length > 20_000 {
            return Err("Form text exceeds 20000 bytes; split the fields into smaller calls.".into());
        }
        parse(field)
    }).collect()
}

impl Browser {
    /// Applies each field once, reads its real result, and stops at the first refusal or failed verification.
    ///
    /// # Errors
    /// Invalid arguments or initial stale refs fail before any field is changed.
    pub async fn fill_form(
        &self,
        session: &str,
        args: &Value,
        by: Actor,
        nav: &dyn Navigate,
        allow_send: bool,
    ) -> Result<Value, String> {
        let actions = parse_form(args)?;
        for action in &actions {
            self.check_ref(session, action)
                .map_err(|error| error.text)?;
        }
        let total = actions.len();
        let mut fields = Vec::new();
        for (index, action) in actions.into_iter().enumerate() {
            let reference = action.reference().unwrap_or_default().to_owned();
            // Owned: `Action::name` borrows the action, which moves into `act_with_sending` below.
            let name = action.name().to_owned();
            match self
                .act_with_sending(session, action, by, nav, allow_send)
                .await
            {
                Ok(receipt) if receipt["ok"] == true => {
                    fields.push(json!({"ref":reference,"action":name,
                    "result":receipt["result"],"target":receipt["untrustedPageContent"]["target"]}));
                }
                Ok(receipt) => {
                    return Ok(json!({"ok":false,"completed":fields.len(),"total":total,
                    "failedAt":index,"blocked":receipt,"untrustedPageContent":{"fields":fields},
                    "note":observe::UNTRUSTED_NOTE,"nextStep":"The field did not return a verified action result; follow the blocked receipt, then read the form again before continuing. Later fields did not run."}));
                }
                Err(error) => {
                    return Ok(json!({"ok":false,"completed":fields.len(),"total":total,
                    "failedAt":index,"error":error.text,"untrustedPageContent":{"fields":fields},
                    "note":observe::UNTRUSTED_NOTE,"nextStep":"The batch stopped at this field; later fields did not run. Earlier changes remain. Read the form again to inspect the failed field and use current refs for any remaining changes."}));
                }
            }
        }
        Ok(
            json!({"ok":true,"completed":fields.len(),"total":total,"untrustedPageContent":{"fields":fields},
            "note":observe::UNTRUSTED_NOTE,"nextStep":"Every field returned its verified result. Submission is a separate action."}),
        )
    }
}
