//! Converts frozen-extension page.read structure into one rendered dataset; never asks for raw execution.
//! Tables require a subtree ref because the fixed row records do not expose table identity.
use serde_json::Value;
use vw_browser::extraction::{Dataset, ExtractKind, ExtractOptions};

pub(super) fn validate(request: &Value, options: &ExtractOptions) -> Result<(), String> {
    if options.selector.is_some()
        || options.index != 0
        || options
            .columns
            .iter()
            .any(|c| c.selector.is_some() || c.attribute.is_some())
    {
        return Err("This browser's captured read has no selector/schema evaluator. Use a page.read subtree ref and positional column names, or the built-in browser's extraction selectors.".into());
    }
    if options.kind == ExtractKind::Table
        && request["args"]["ref"].as_str().is_none()
        && request["args"]["cursor"].as_str().is_none()
    {
        return Err("A table export needs one table subtree. Read with interactiveOnly:false to get its ref, then supply args.ref; continue its captured rows with the returned cursor.".into());
    }
    if options.kind == ExtractKind::Table
        && request["args"]["cursor"].is_string()
        && options.header.is_none()
    {
        return Err("A continuation may start with a data row; set extract.header:false, or true only when its first row is a header, and provide the collection's column names.".into());
    }
    Ok(())
}

pub(super) fn extract(body: &Value, options: &ExtractOptions) -> Result<Dataset, String> {
    let data = &body["untrustedPageContent"]["data"];
    let mut rows = Vec::new();
    match options.kind {
        ExtractKind::Table | ExtractKind::List => {
            for section in data["structure"].as_array().into_iter().flatten() {
                let key = if options.kind == ExtractKind::Table {
                    "tables"
                } else {
                    "lists"
                };
                for group in section[key].as_array().into_iter().flatten() {
                    if options.kind == ExtractKind::Table {
                        for row in group["rows"].as_array().into_iter().flatten() {
                            rows.push(
                                row.as_array()
                                    .ok_or("Captured table row is malformed; make a fresh read.")?
                                    .iter()
                                    .map(|cell| {
                                        cell.as_str()
                                            .map(str::to_owned)
                                            .ok_or("Captured cell is not text; make a fresh read.")
                                    })
                                    .collect::<Result<Vec<_>, _>>()?,
                            );
                        }
                    } else {
                        for item in group["items"].as_array().into_iter().flatten() {
                            rows.push(vec![
                                item.as_str()
                                    .ok_or("Captured list item is not text; make a fresh read.")?
                                    .to_owned(),
                            ]);
                        }
                    }
                }
            }
        }
        ExtractKind::Links => {
            for item in data["refs"].as_array().into_iter().flatten() {
                if let Some(href) = item["href"].as_str() {
                    rows.push(vec![
                        item["name"].as_str().unwrap_or_default().into(),
                        href.into(),
                    ]);
                }
            }
        }
    }
    let mut columns = match options.kind {
        ExtractKind::Table if options.header.unwrap_or(true) && !rows.is_empty() => rows.remove(0),
        ExtractKind::Table => (0..rows.first().map_or(0, Vec::len))
            .map(|i| format!("Column {}", i + 1))
            .collect(),
        ExtractKind::List => vec!["Text".into()],
        ExtractKind::Links => vec!["Text".into(), "URL".into()],
    };
    if !options.columns.is_empty() {
        columns = options.columns.iter().map(|c| c.name.clone()).collect();
    }
    Ok(Dataset {
        columns,
        rows,
        source: data["url"].as_str().unwrap_or_default().into(),
        truncated: data["coverage"]["complete"] != true
            || data["scanTruncated"] == true
            || data["nextOffset"].is_number()
            || data["nextTextOffset"].is_number(),
    })
}
