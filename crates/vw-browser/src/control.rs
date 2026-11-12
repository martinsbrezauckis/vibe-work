//! Bounded focus and selection-preserving edits; parsed by `browser_act` and rendered by the checked page script.
use serde_json::{Value, json};
/// One control operation bound to an observed element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    name: &'static str,
    reference: String,
    text: Option<String>,
    clear: bool,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let name = match args["action"].as_str() {
            Some("focus") => "focus",
            Some("blur") => "blur",
            Some("highlight") => "highlight",
            _ => "insert_text",
        };
        let reference = args["ref"]
            .as_str()
            .filter(|r| crate::model::is_ref(r))
            .ok_or("This action needs a current element ref. Read the page and choose its ref.")?
            .to_owned();
        let text = if name == "insert_text" {
            Some(args["text"].as_str().filter(|v| v.chars().count() <= crate::act::MAX_TEXT && !v.contains('\0'))
                .ok_or("Insertion needs text of at most 10000 characters. Shorten the text and try again.")?.to_owned())
        } else {
            None
        };
        if args["submit"] == true {
            return Err("Insertion does not submit the field. Send a separate Enter after checking its value.".into());
        }
        Ok(Self {
            name,
            reference,
            text,
            clear: args["clear"] == true,
        })
    }
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    pub(crate) fn reference(&self) -> &str {
        &self.reference
    }
    pub(crate) fn args(&self) -> Value {
        json!({"ref":self.reference,"text":self.text,"clear":self.clear})
    }
}
