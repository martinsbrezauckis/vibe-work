//! The page reader's part for inputs (Handbook Format 1.2 §6.6): an element with `data-input` starts an
//! input; `data-option` elements and the `.hb-question` inside it add themselves when they end; markup problems are
//! collected for the validator (HB-E10, HB-W17). Split out of `page_scan`.
//!
//! Depends on: `page_scan` (the open-element stack), `page` (types), `html`, `handbook::format`. Called by:
//! `page_scan`.

use crate::handbook::format::{INPUT_TYPES, MAX_INPUTS, MAX_OPTIONS};
use crate::html::{Tag, clip};
use crate::page::{Input, InputOption, PageDetail, PageFacts, PageScan};
use crate::page_scan::Open;

/// An element with `data-input` starts an input (§6.6): records it (with its markup problems) and returns its index.
pub(crate) fn open_input(tag: &Tag, stack: &[Open], scan: &mut PageScan) -> Option<usize> {
    let kind = tag.attr("data-input")?.trim().to_ascii_lowercase();
    let detail = &mut scan.detail;
    if stack.iter().any(|o| o.input.is_some()) {
        detail
            .input_problems
            .push("an input inside another input".to_owned());
        return None;
    }
    let Some(id) = tag
        .attr("id")
        .map(|v| clip(v.trim(), 100))
        .filter(|v| !v.is_empty())
    else {
        detail.input_problems.push(format!(
            "a {} input has no id (answers are kept by the input's id)",
            clip(&kind, 20)
        ));
        return None;
    };
    if scan.facts.inputs.len() >= MAX_INPUTS {
        if !detail
            .input_warnings
            .iter()
            .any(|w| w.starts_with("more than"))
        {
            detail.input_warnings.push(format!(
                "more than {MAX_INPUTS} inputs: the rest are not read"
            ));
        }
        return None;
    }
    if !INPUT_TYPES.contains(&kind.as_str()) {
        detail.unknown_input_types.push(clip(&kind, 40));
    }
    scan.facts.inputs.push(Input {
        id,
        kind,
        question: String::new(),
        options: Vec::new(),
    });
    Some(scan.facts.inputs.len() - 1)
}

/// An element ended: an option or question adds itself to the enclosing input; an input checks its options.
pub(crate) fn close_input_part(
    open: &Open,
    text: &str,
    stack: &[Open],
    facts: &mut PageFacts,
    detail: &mut PageDetail,
) {
    let owner = stack.iter().rev().find_map(|o| o.input);
    if let Some(value) = &open.option {
        match owner.and_then(|i| facts.inputs.get_mut(i)) {
            None => detail.input_problems.push(format!(
                "data-option \"{}\" outside an input",
                clip(value, 40)
            )),
            Some(input) if value.is_empty() => detail.input_problems.push(format!(
                "an option of input {} has an empty data-option",
                input.id
            )),
            Some(input) if input.options.iter().any(|o| o.id == *value) => {
                detail.input_problems.push(format!(
                    "option \"{}\" is used twice in input {}",
                    clip(value, 40),
                    input.id
                ));
            }
            Some(input) if input.options.len() >= MAX_OPTIONS => {
                if !detail.input_warnings.iter().any(|w| w.contains(&input.id)) {
                    detail.input_warnings.push(format!(
                        "input {} has more than {MAX_OPTIONS} options: the rest are not read",
                        input.id
                    ));
                }
            }
            Some(input) => input.options.push(InputOption {
                id: value.clone(),
                label: clip(text, 200),
            }),
        }
    }
    if open.question
        && let Some(input) = owner.and_then(|i| facts.inputs.get_mut(i))
        && input.question.is_empty()
    {
        input.question = clip(text, 300);
    }
    let Some(input) = open.input.and_then(|i| facts.inputs.get(i)) else {
        return;
    };
    if input.question.is_empty() {
        detail.input_warnings.push(format!(
            "input {} has no question (p.hb-question)",
            input.id
        ));
    }
    match (input.kind.as_str(), input.options.is_empty()) {
        ("choice" | "checklist" | "rank", true) => detail.input_problems.push(format!(
            "{} input {} has no options (li data-option)",
            input.kind, input.id
        )),
        ("text", false) => detail.input_warnings.push(format!(
            "text input {} has options; a text answer ignores them",
            input.id
        )),
        _ => {}
    }
}
