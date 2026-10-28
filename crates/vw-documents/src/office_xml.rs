//! Bounded OOXML parts and XML trees shared by the local Word/PowerPoint readers.
//! Depends on zipinfo and quick-xml (already used by calamine). Never resolves DTDs or external links.

use quick_xml::{Reader, events::Event};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Node {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<Self>,
}

impl Node {
    pub fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map_or("", String::as_str)
    }
    pub fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.name == name)
    }
    pub fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Self>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.descendants(name, out);
        }
    }
    pub fn all(&self, name: &str) -> Vec<&Self> {
        let mut nodes = Vec::new();
        self.descendants(name, &mut nodes);
        nodes
    }
    pub fn value(&self, name: &str) -> &str {
        self.all(name).first().map_or("", |n| n.attr("val"))
    }
    pub fn inline(&self, out: &mut String) -> Result<(), String> {
        use crate::text_budget::append;
        match self.name.as_str() {
            "t" => append(out, &self.text),
            "tab" => append(out, "\t"),
            "br" | "cr" => append(out, "\n"),
            // Deleted text and field instructions are not displayed document text.
            "del" | "instrText" => Ok(()),
            _ => {
                for child in &self.children {
                    child.inline(out)?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => {
                count += 1;
                if stack.len() > 128 || count > 1_000_000 {
                    return Err("XML is too complex".to_owned());
                }
                let mut node = Node {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    ..Node::default()
                };
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| e.to_string())?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = attr
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    node.attrs.insert(key.clone(), value.clone());
                    if let Some((_, local)) = key.split_once(':') {
                        node.attrs.entry(local.to_owned()).or_insert(value);
                    }
                }
                stack.push(node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or("invalid XML nesting")?;
                stack
                    .last_mut()
                    .ok_or("invalid XML nesting")?
                    .children
                    .push(node);
            }
            Event::Text(e) => stack
                .last_mut()
                .ok_or("invalid XML")?
                .text
                .push_str(&e.decode().map_err(|e| e.to_string())?),
            Event::CData(e) => stack
                .last_mut()
                .ok_or("invalid XML")?
                .text
                .push_str(&e.decode().map_err(|e| e.to_string())?),
            Event::GeneralRef(e) => {
                let value = if let Some(c) = e.resolve_char_ref().map_err(|e| e.to_string())? {
                    c.to_string()
                } else {
                    quick_xml::escape::resolve_xml_entity(&e.decode().map_err(|e| e.to_string())?)
                        .ok_or("unknown XML entity")?
                        .to_owned()
                };
                stack.last_mut().ok_or("invalid XML")?.text.push_str(&value);
            }
            Event::DocType(_) => return Err("XML document types are not supported".to_owned()),
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Err("incomplete XML".to_owned());
    }
    let mut root = stack.pop().ok_or("empty XML")?;
    if root.children.len() != 1 {
        return Err("XML must have one root".to_owned());
    }
    root.children.pop().ok_or_else(|| "empty XML".to_owned())
}

pub(crate) fn part(bytes: &[u8], name: &str) -> Result<Node, String> {
    let xml =
        crate::zipinfo::entry_text_within(bytes, name, crate::zipinfo::MAX_CONTENT_PART_BYTES)
            .ok_or_else(|| format!("missing, oversized or unreadable part: {name}"))?;
    parse(&xml)
}

/// Resolve package-internal relationship targets, never filesystem paths or network URLs.
pub(crate) fn related(
    bytes: &[u8],
    source: &str,
) -> Result<BTreeMap<String, (String, String)>, String> {
    let (folder, name) = source.rsplit_once('/').ok_or("invalid part name")?;
    let rels = format!("{folder}/_rels/{name}.rels");
    if !crate::zipinfo::entry_names(bytes)?.contains(&rels) {
        return Ok(BTreeMap::new());
    }
    let root = part(bytes, &rels)?;
    let mut out = BTreeMap::new();
    for rel in root.all("Relationship") {
        if rel.attr("TargetMode") == "External" {
            continue;
        }
        let target = rel.attr("Target");
        let mut parts: Vec<_> = if target.starts_with('/') {
            Vec::new()
        } else {
            folder.split('/').collect()
        };
        for p in target.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    parts.pop().ok_or("relationship leaves the package")?;
                }
                _ => parts.push(p),
            }
        }
        out.insert(
            rel.attr("Id").to_owned(),
            (parts.join("/"), rel.attr("Type").to_owned()),
        );
    }
    Ok(out)
}
