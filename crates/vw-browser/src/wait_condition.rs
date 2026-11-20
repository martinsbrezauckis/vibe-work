//! Bounded read-only DOM/address waits, preserving observed frame bindings; called by the page action driver.
use crate::driver::{failed, refused};
use crate::{ActError, Browser};
use serde_json::{Value, json};
use std::time::Duration;
/// A validated condition or plain delay, separate from input and executable page scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    args: Value,
    timeout: u64,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        if args.get("networkIdle").is_some() {
            return Err("The built-in browser does not expose complete network activity on every OS. Wait for a page element, text, address or load state instead.".into());
        }
        if args.get("expression").is_some() {
            return Err("The built-in wait does not execute arbitrary page scripts. Use a selector, ref, text, address or load state condition.".into());
        }
        let timeout = args
            .get("timeoutMs")
            .map_or(Some(crate::act::DEFAULT_WAIT_MS), Value::as_u64)
            .filter(|n| (1..=crate::act::MAX_WAIT_MS).contains(n))
            .ok_or("Wait timeout must be 1–15000 milliseconds. Choose a bounded timeoutMs.")?;
        let count = ["selector", "ref", "url", "load", "ms", "text"]
            .iter()
            .filter(|key| args.get(**key).is_some())
            .count();
        if count != 1 {
            return Err("Wait needs exactly one condition or a plain ms delay. Choose a ref, selector, text, url or load state.".into());
        }
        let mut data = json!({});
        for (name, limit) in [("selector", 2000), ("url", 8192), ("ref", 100)] {
            if let Some(value) = args.get(name) {
                let value = value
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= limit && !s.contains('\0'))
                    .ok_or(
                        "The wait target is invalid or too long. Choose a bounded target string.",
                    )?;
                if name == "selector" && tests_field_values(value) {
                    return Err(VALUE_SELECTOR.into());
                }
                if name == "ref" && !crate::model::is_ref(value) {
                    return Err(
                        "Wait needs a current observed ref. Read the page and choose its ref."
                            .into(),
                    );
                }
                data[name] = json!(value);
            }
        }
        if let Some(state) = args.get("state") {
            if !matches!(
                state.as_str(),
                Some("attached" | "detached" | "visible" | "hidden")
            ) || (data.get("ref").is_none() && data.get("selector").is_none())
            {
                return Err("An element state needs ref or selector. Choose attached, detached, visible or hidden.".into());
            }
            data["state"] = state.clone();
        }
        if let Some(load) = args.get("load") {
            if !matches!(load.as_str(), Some("interactive" | "complete")) && load != true {
                return Err("Load waits support interactive, complete or true. Choose a supported readiness state.".into());
            }
            data["load"] = if load == true {
                json!("complete")
            } else {
                load.clone()
            };
        }
        if let Some(ms) = args.get("ms") {
            if args.get("timeoutMs").is_some()
                || ms.as_u64().is_none_or(|n| !(1..=10000).contains(&n))
            {
                return Err("A plain delay needs ms from 1 to 10000 and no timeoutMs. Choose a bounded delay.".into());
            }
            data["ms"] = ms.clone();
        }
        Ok(Self {
            args: data,
            timeout,
        })
    }
    pub(crate) fn reference(&self) -> Option<&str> {
        self.args["ref"].as_str()
    }
}
impl Browser {
    pub(crate) async fn wait_condition(
        &self,
        session: &str,
        request: &Request,
    ) -> Result<Value, ActError> {
        if let Some(ms) = request.args["ms"].as_u64() {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            return Ok(json!({"action":"wait","met":true,"elapsedMs":ms}));
        }
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_millis(request.timeout);
        let mount = request
            .reference()
            .map(|_| self.ref_mount(session))
            .transpose()?;
        loop {
            let probe = async {
                if mount.is_some() && self.service.page_mount(session) != mount {
                    return Err(refused(crate::refs::PAGE_CLOSED));
                }
                let page = if mount.is_some() {
                    self.service
                        .page_accessor(session)
                        .ok_or_else(|| refused(crate::refs::PAGE_CLOSED))?
                } else {
                    self.page(session).await.map_err(refused)?
                };
                let mut args = request.args.clone();
                args["page"] = json!(page);
                self.bind_ref_request(session, &mut args);
                let answer: Value = self
                    .run_in_mount(
                        session,
                        crate::scripts::wait_condition(&args),
                        mount.as_ref(),
                    )
                    .await
                    .map_err(failed)?;
                self.reading_ref_allowed(
                    session,
                    &page,
                    answer["url"].as_str().unwrap_or_default(),
                    request.reference(),
                )
                .map_err(refused)?;
                if answer["invalid"] == true {
                    return Err(refused(
                        "The selector is invalid or the observed element exceeds the scan bound. Use a valid CSS selector or read a more specific ref.",
                    ));
                }
                Ok(answer["met"] == true)
            };
            let met = match tokio::time::timeout_at(deadline, probe).await {
                Ok(result) => result?,
                Err(_) => false,
            };
            if met || tokio::time::Instant::now() >= deadline {
                return Ok(
                    json!({"action":"wait","met":met,"elapsedMs":started.elapsed().as_millis(),"condition":request.args,"note":if met {"condition observed"}else{"condition not met before timeout"}}),
                );
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_millis(100)).min(deadline),
            )
            .await;
        }
    }
}

/// A wait matches the live page, so a selector that tests a field's value would answer questions about it one wait at a
/// time (`#card[value^="4"]`, also inside `:has()`), including a card number, PIN or password. Such selectors are
/// refused on every page: the wait cannot know the page before it runs.
pub(crate) const VALUE_SELECTOR: &str = "This wait selector tests a field's value or validity. Wait for an element, its visibility or a class instead; read field values with page.read.";

/// Attributes a wait selector may compare: element identity and labels, never a field's state.
const IDENTITY_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "name",
    "type",
    "role",
    "for",
    "href",
    "lang",
    "dir",
    "title",
    "alt",
    "aria-label",
    "aria-labelledby",
    "aria-describedby",
    "aria-controls",
    "aria-expanded",
    "aria-selected",
    "aria-hidden",
    "aria-disabled",
    "data-testid",
    "data-test-id",
    "data-test",
    "data-qa",
    "data-cy",
];

/// Whether a CSS selector may test a field value: any `value` attribute test, a comparison on an attribute outside
/// [`IDENTITY_ATTRIBUTES`], or a value-dependent pseudo-class, after CSS
/// escapes are resolved; any comment is refused.
pub(crate) fn tests_field_values(selector: &str) -> bool {
    // A comment can hide part of a selector inside a quoted string, where it is not a comment: refused outright.
    if selector.contains("/*") {
        return true;
    }
    let mut plain = String::with_capacity(selector.len());
    let mut chars = selector.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            plain.extend(c.to_lowercase());
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.push(chars.next().unwrap_or('0'));
        }
        if hex.is_empty() {
            if let Some(next) = chars.next() {
                plain.extend(next.to_lowercase());
            }
            continue;
        }
        if chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let decoded = u32::from_str_radix(&hex, 16)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\u{fffd}');
        plain.extend(decoded.to_lowercase());
    }
    let squeezed: String = plain.chars().filter(|c| !c.is_whitespace()).collect();
    let attribute = squeezed.match_indices('[').any(|(at, _)| {
        let rest = &squeezed[at + 1..];
        // An optional namespace prefix (`ns|value`, `*|value`, `|value`), never the `|=` operator.
        let name = match rest.find('|') {
            Some(bar)
                if !rest[bar + 1..].starts_with('=')
                    && rest[..bar]
                        .chars()
                        .all(|c| c == '*' || c == '-' || c == '_' || c.is_alphanumeric()) =>
            {
                &rest[bar + 1..]
            }
            _ => rest,
        };
        let end = name
            .find([']', '=', '~', '|', '^', '$', '*'])
            .unwrap_or(name.len());
        let (attribute, operator) = (&name[..end], !name[end..].starts_with(']'));
        // A page can mirror a field's value into any attribute: only identity attributes may be compared, and the
        // value attribute may not even be tested for presence.
        attribute == "value" || operator && !IDENTITY_ATTRIBUTES.contains(&attribute)
    });
    // A custom element state (`:state(...)`) is whatever the page sets, a field's value included.
    attribute
        || squeezed.contains(":state(")
        || [
            ":placeholder-shown",
            ":valid",
            ":invalid",
            ":user-valid",
            ":user-invalid",
            ":in-range",
            ":out-of-range",
            ":blank",
            ":empty",
            ":-moz-placeholder",
            ":-moz-ui-invalid",
            ":-moz-ui-valid",
            ":autofill",
            ":-webkit-autofill",
        ]
        .iter()
        .any(|pseudo| {
            squeezed.match_indices(pseudo).any(|(at, _)| {
                !squeezed[at + pseudo.len()..]
                    .starts_with(|c: char| c == '-' || c.is_alphanumeric())
            })
        })
}

#[cfg(test)]
mod value_selector_tests {
    // PREDICT (before the fix): a selector wait that tests a field's value is admitted, so `#card[value^="4"]` waits
    // answer a protected value one character at a time.
    #[test]
    fn selector_waits_that_test_field_values_are_refused() {
        for selector in [
            "#card[value^=\"4\"]",
            "form:has(#card[value*=\"0566\"])",
            "#pin[*|value=\"8812\"]",
            "#pin[value|=\"88\"]",
            "#pin[v\\61lue^=\"8\"]",
            "#pin[val/**/ue]",
            "[data-x=\"/*\"] #card[value^=\"4\"] [data-y=\"*/\"]",
            "#card:placeholder-shown",
            "#pin:INVALID",
            "span[contenteditable]:empty",
            "[value]",
            "[data-value=\"4000\"]",
            "[aria-valuetext*=\"4\"]",
            "#card:-webkit-autofill",
            "pay-field:state(filled)",
            "#pin[data-state=filled]",
        ] {
            let refused = super::Request::parse(&serde_json::json!({ "selector": selector }));
            assert_eq!(
                refused.err().as_deref(),
                Some(super::VALUE_SELECTOR),
                "{selector}"
            );
        }
        for selector in [
            "#order-confirmed",
            "input[name=card]",
            "[data-testid=done]",
            "input[disabled]",
            "button:enabled",
            "#valid",
        ] {
            assert!(
                super::Request::parse(&serde_json::json!({ "selector": selector })).is_ok(),
                "{selector}"
            );
        }
    }
}
