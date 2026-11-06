//! Rendered terms around page choices, independent of find and text paging.
//! Used by observation, frame merging and output fitting; copies page facts without classifying or choosing offers.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A bounded excerpt of an overlay or covering document, always untrusted page content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceContext {
    /// Owning document's origin; the native merger supplies verified child origins.
    pub frame_origin: String,
    /// Rendered headings keep offer names and prices even when the body excerpt is shortened.
    pub headings: Vec<String>,
    /// Rendered supporting terms, separate from the ordinary text window.
    pub text: String,
    /// Whether both lists and text fit without omissions.
    pub complete: bool,
}

impl ChoiceContext {
    pub(crate) fn bound(&mut self) {
        let cap = |text: &mut String, max: usize| {
            text.retain(|c| !crate::text::hidden(c));
            if text.chars().count() > max {
                *text = text.chars().take(max).collect();
                true
            } else {
                false
            }
        };
        cap(&mut self.frame_origin, 300);
        self.complete &= self.headings.len() <= 8;
        self.headings.truncate(8);
        for heading in &mut self.headings {
            self.complete &= !cap(heading, 180);
        }
        self.complete &= !cap(&mut self.text, 1000);
    }
}

pub(crate) fn include(content: &mut Value, model: &crate::model::PageModel) {
    if !model.choice_context.is_empty() {
        content["choiceContext"] = json!(model.choice_context);
    }
}

/// Supporting prose yields before headings; headings stay exact or are explicitly omitted.
pub(crate) fn trim(content: &mut Value, headings: bool) -> bool {
    let Some(contexts) = content
        .get_mut("choiceContext")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    for context in contexts.iter_mut().rev() {
        let text = context["text"].as_str().unwrap_or("");
        if !text.is_empty() {
            context["text"] = json!(
                text.chars()
                    .take(text.chars().count() / 2)
                    .collect::<String>()
            );
            context["complete"] = json!(false);
            return true;
        }
        if headings
            && context["headings"]
                .as_array_mut()
                .is_some_and(|items| items.pop().is_some())
        {
            context["complete"] = json!(false);
            return true;
        }
    }
    if headings && !contexts.is_empty() {
        contexts.pop();
        content["choiceContextOmitted"] =
            json!(content["choiceContextOmitted"].as_u64().unwrap_or(0) + 1);
        return true;
    }
    false
}
