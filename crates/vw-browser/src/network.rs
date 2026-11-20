//! Bounded native request reads; URLs are stripped of query values and headers default to safe metadata.
//! Shared core classification reserves unredacted headers for Full access; vault-filled documents refuse them.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Request-list filters or one request's header read.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    action: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    last: Option<usize>,
    url: Option<String>,
    method: Option<String>,
    status: Option<u16>,
    #[serde(default)]
    failed_only: bool,
    #[serde(rename = "requestId")]
    id: Option<String>,
    #[serde(default)]
    unredacted: bool,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let request:Self=serde_json::from_value(args.clone()).map_err(|_|"Request diagnostics need typed filters; unredacted and failedOnly must be booleans.")?;
        if request.offset > 20000
            || request.limit.is_some_and(|n| !(1..=100).contains(&n))
            || request.last.is_some_and(|n| !(1..=100).contains(&n))
            || request.last.is_some() && args.get("offset").is_some()
            || request.url.as_ref().is_some_and(|s| s.len() > 2000)
            || request.method.as_ref().is_some_and(|s| {
                s.is_empty() || s.len() > 16 || !s.bytes().all(|b| b.is_ascii_alphabetic())
            })
        {
            return Err("Request filters exceed their limits; use offset up to 20000, limit/last 1–100 and a method name.".into());
        }
        if request.action == "network.headers"
            && request
                .id
                .as_ref()
                .is_none_or(|s| s.is_empty() || s.len() > 128)
        {
            return Err(
                "Header reads need requestId from network.list for the current document.".into(),
            );
        }
        Ok(request)
    }
    pub(crate) fn name(&self) -> &'static str {
        if self.action == "network.headers" {
            "network.headers"
        } else {
            "network.list"
        }
    }
    pub(crate) fn unredacted_headers(&self) -> bool {
        self.action == "network.headers" && self.unredacted
    }
    fn result(&self, snapshot: &Value) -> Result<Value, String> {
        let rows = snapshot["requests"].as_array().ok_or(
            "The engine returned no request records; reload the page before reading requests.",
        )?;
        if self.action == "network.headers" {
            let row=rows.iter().find(|r|r["requestId"].as_str()==self.id.as_deref()).ok_or("This request expired or belongs to another document; use a current requestId from network.list.")?;
            return Ok(headers(row, self.unredacted));
        }
        let matching: Vec<_> = rows
            .iter()
            .filter(|r| {
                (!self.failed_only
                    || r.get("error").is_some()
                    || r["status"].as_u64().is_some_and(|s| s >= 400))
                    && self
                        .url
                        .as_ref()
                        .is_none_or(|s| clean_url(&r["url"]).contains(s))
                    && self.method.as_ref().is_none_or(|s| {
                        r["method"]
                            .as_str()
                            .is_some_and(|m| m.eq_ignore_ascii_case(s))
                    })
                    && self
                        .status
                        .is_none_or(|s| r["status"].as_u64() == Some(u64::from(s)))
            })
            .collect();
        let total = matching.len();
        let offset = self.last.map_or(self.offset, |n| total.saturating_sub(n));
        let mut entries = Vec::new();
        let mut bytes = 0;
        for row in matching
            .into_iter()
            .skip(offset)
            .take(self.last.or(self.limit).unwrap_or(25))
        {
            let mut item = json!({"requestId":row["requestId"],"url":clean_url(&row["url"]),"method":row["method"],"status":row["status"],"finished":row["finished"]});
            if let Some(error) = row["error"].as_str() {
                item["error"] = json!(error.chars().take(220).collect::<String>());
            }
            if bytes + item.to_string().len() > 1700 {
                break;
            }
            bytes += item.to_string().len();
            entries.push(item);
        }
        let next = offset + entries.len();
        Ok(
            json!({"requests":entries,"total":total,"offset":offset,"nextOffset":(next<total).then_some(next),"dropped":snapshot["dropped"],"coverage":snapshot["coverage"]}),
        )
    }
}
fn clean_url(value: &Value) -> String {
    let Ok(mut url) = url::Url::parse(value.as_str().unwrap_or_default()) else {
        return "[unavailable URL]".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    if url.query().is_some() {
        url.set_query(Some("[redacted]"));
    }
    url.as_str().chars().take(400).collect()
}
fn headers(row: &Value, raw: bool) -> Value {
    let mut out = json!({"requestId":row["requestId"],"unredacted":raw,"truncated":row["requestHeadersTruncated"]==true || row["responseHeadersTruncated"]==true});
    let mut bytes = 0;
    for (source, destination) in [
        ("requestHeaders", "request"),
        ("responseHeaders", "response"),
    ] {
        let mut headers = serde_json::Map::new();
        if let Some(map) = row[source].as_object() {
            for (name, value) in map {
                let safe = matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-type"
                        | "content-length"
                        | "content-encoding"
                        | "cache-control"
                        | "accept"
                        | "accept-encoding"
                );
                let value = if raw || safe {
                    value.as_str().unwrap_or_default()
                } else {
                    "[redacted]"
                };
                let size = name.len() + value.len() + 8;
                if bytes + size > 1700 {
                    out["truncated"] = json!(true);
                    continue;
                }
                bytes += size;
                headers.insert(name.clone(), json!(value));
            }
        }
        out[destination] = json!(headers);
    }
    out
}

/// A policy refusal comes from Vibe Work's network guard, not the page's native request log.
/// Keep it beside the untrusted rows. Correlate a native 403 or error only when it is the sole
/// eligible row for that target in the complete current-document log and both refusal reads agree.
/// A native row must also have started after the refusal's millisecond. Otherwise leave it unpaired.
fn network_receipt(
    action: &str,
    data: &Value,
    snapshot: &Value,
    host_refusals: &[Value],
    pairable_count: usize,
) -> Value {
    let mut data = data.clone();
    let mut refusals = host_refusals.to_vec();
    if let (Some(rows), Some(native)) = (
        data["requests"].as_array_mut(),
        snapshot["requests"].as_array(),
    ) && snapshot["dropped"].as_u64().unwrap_or(0) == 0
        && pairable_count == host_refusals.len()
    {
        let mut marked = vec![false; native.len()];
        for (index, refusal) in refusals.iter_mut().enumerate() {
            let Some(target) = refusal["target"].as_str() else {
                continue;
            };
            let Some(refused_at) = refusal["atMs"].as_u64() else {
                continue;
            };
            let mut candidates = native.iter().enumerate().filter(|(at, row)| {
                !marked[*at]
                    && (row["status"] == 403 || row["error"].is_string())
                    && row["startedAtMs"]
                        .as_u64()
                        .is_some_and(|start| start > refused_at)
                    && row_target(row).is_some_and(|t| t.eq_ignore_ascii_case(target))
            });
            let Some((at, candidate)) = candidates.next() else {
                continue;
            };
            if candidates.next().is_some() {
                continue;
            }
            let Some(id) = candidate["requestId"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let Some(row) = rows
                .iter_mut()
                .find(|row| row["requestId"].as_str() == Some(id))
            else {
                continue;
            };
            marked[at] = true;
            row["refusedBy"] = json!("Vibe Work");
            row["refusal"] = json!(index);
            refusal["requestId"] = json!(id);
        }
    }
    let mut receipt = json!({"ok":true,"action":action,"Outcome":"Native request records read.","untrustedPageContent":data});
    if !refusals.is_empty() {
        receipt["networkRequestRefusals"] = json!(refusals);
    }
    receipt
}

/// `host:port` of a native row's URL, in the refusal's `target` form (IPv6 in brackets).
fn row_target(row: &Value) -> Option<String> {
    let url = url::Url::parse(row["url"].as_str()?).ok()?;
    Some(format!(
        "{}:{}",
        url.host_str()?,
        url.port_or_known_default()?
    ))
}

fn pairable_refusal_count(
    before_mark: u64,
    before_refusals: &[Value],
    after_mark: u64,
    after_refusals: &[Value],
) -> usize {
    if before_mark == after_mark && before_refusals == after_refusals {
        after_refusals.len()
    } else {
        0
    }
}
impl Browser {
    pub(crate) async fn network(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        if request.unredacted && self.service.shots_blocked(session) {
            return Err(refused(
                "Unredacted headers are blocked after a vault login because they could contain credentials; read redacted headers or open a fresh document.",
            ));
        }
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let before_mark = self.service.request_refusal_mark(session);
        let before_refusals = if request.name() == "network.list" {
            self.service.request_refusals_on_page(session)
        } else {
            Vec::new()
        };
        let snapshot = self
            .service
            .network_records(mount)
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let data = request.result(&snapshot).map_err(refused)?;
        let text = data.to_string();
        let text = vw_secrets::scrub(text.as_bytes())
            .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
        let data: Value = serde_json::from_str(&text).map_err(|_| {
            failed("Request details could not be redacted; read the page before continuing.")
        })?;
        let host_refusals = if request.name() == "network.list" {
            self.service.request_refusals_on_page(session)
        } else {
            Vec::new()
        };
        let after_mark = self.service.request_refusal_mark(session);
        let pairable_count =
            pairable_refusal_count(before_mark, &before_refusals, after_mark, &host_refusals);
        Ok(network_receipt(
            request.name(),
            &data,
            &snapshot,
            &host_refusals,
            pairable_count,
        ))
    }
}

#[cfg(test)]
mod host_refusal_tests {
    use serde_json::json;

    /// PREDICT: a refusal first observed after the native snapshot labels the earlier server 403.
    #[test]
    fn a_refusal_arriving_after_native_snapshot_must_not_label_an_unrelated_403() {
        let snapshot = json!({"requests":[
            {"requestId":"real-early","url":"http://10.55.5.17:18115/real","status":403,"startedAtMs":40}
        ],"dropped":0});
        let before: [serde_json::Value; 0] = [];
        let after = [json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"})];
        // The reads are injected in the same order as network.list: before, native, after.
        let pairable_count = super::pairable_refusal_count(0, &before, 1, &after);
        let receipt =
            super::network_receipt("network.list", &snapshot, &snapshot, &after, pairable_count);
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["reason"],
            "sender unknown"
        );
    }

    /// PREDICT: a policy refusal may precede its own native callback; the sole older 403 is still mislabeled.
    #[test]
    fn a_refusal_without_its_native_row_must_not_label_an_earlier_403() {
        let snapshot = json!({"requests":[
            {"requestId":"real-early","url":"http://10.55.5.17:18115/real","status":403,"startedAtMs":40}
        ],"dropped":0});
        let refusal = json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"});
        let receipt = super::network_receipt("network.list", &snapshot, &snapshot, &[refusal], 1);
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
        assert!(
            receipt["networkRequestRefusals"][0]
                .get("requestId")
                .is_none()
        );
    }

    #[test]
    fn a_new_refusal_cannot_pair_even_when_a_candidate_start_time_is_later() {
        let snapshot = json!({"requests":[
            {"requestId":"other-403","url":"http://10.55.5.17:18115/real","status":403,"startedAtMs":43}
        ],"dropped":0});
        let before: [serde_json::Value; 0] = [];
        let after = [json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"})];
        let pairable_count = super::pairable_refusal_count(0, &before, 1, &after);
        let receipt =
            super::network_receipt("network.list", &snapshot, &snapshot, &after, pairable_count);
        assert_eq!(pairable_count, 0);
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
        assert!(
            receipt["networkRequestRefusals"][0]
                .get("requestId")
                .is_none()
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["reason"],
            "sender unknown"
        );
    }

    #[test]
    fn a_row_without_a_comparable_start_time_stays_unpaired() {
        let snapshot = json!({"requests":[
            {"requestId":"server","url":"http://10.55.5.17:18115/real","status":403}
        ],"dropped":0});
        let refusal = json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"});
        let receipt = super::network_receipt("network.list", &snapshot, &snapshot, &[refusal], 1);
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
    }

    #[test]
    fn same_millisecond_does_not_prove_a_row_started_after_a_refusal() {
        let snapshot = json!({"requests":[
            {"requestId":"server","url":"http://10.55.5.17:18115/real","status":403,"startedAtMs":42}
        ],"dropped":0});
        let refusal = json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"});
        let receipt = super::network_receipt("network.list", &snapshot, &snapshot, &[refusal], 1);
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
    }

    /// PREDICT (fails before the change): the native row of a request Vibe Work refused showed only status 403, so an
    /// agent read it as the server's answer. The row now names Vibe Work and its refusal row; the refusal names the row.
    #[test]
    fn the_native_403_of_a_refused_request_says_vibe_work_refused_it() {
        let snapshot = json!({"requests":[
            {"requestId":"req-1","url":"https://example.com/","method":"GET","status":200,"finished":true},
            {"requestId":"req-5","url":"http://10.55.5.17:18115/r20.png?[redacted]","method":"GET","status":403,"finished":true,"startedAtMs":43},
            {"requestId":"req-6","url":"http://10.55.5.17:18115/other","method":"GET","status":200,"finished":true}
        ]});
        let receipt = super::network_receipt(
            "network.list",
            &snapshot,
            &snapshot,
            &[json!({
                "atMs": 42,
                "origin": null,
                "target": "10.55.5.17:18115",
                "targetClass": "local-network",
                "reason": "sender unknown",
                "senderKnown": false,
                "message": "Browser request refused by Vibe Work: several web pages are open in the browser and this request does not say which one sent it"
            })],
            1,
        );
        let rows = &receipt["untrustedPageContent"]["requests"];
        assert_eq!(rows[1]["refusedBy"], "Vibe Work", "{receipt}");
        assert_eq!(rows[1]["refusal"], 0);
        assert_eq!(receipt["networkRequestRefusals"][0]["requestId"], "req-5");
        // One native row per refusal row; an answer of the page's own server is never marked.
        assert!(rows[2].get("refusedBy").is_none(), "{receipt}");
        assert!(rows[0].get("refusedBy").is_none(), "{receipt}");
    }

    /// PREDICT: two eligible rows for one target remain unpaired because the refusal does not identify either request.
    #[test]
    fn a_real_403_before_a_refusal_stays_unmarked() {
        let snapshot = json!({"requests": [
            {"requestId":"server-403","url":"http://10.55.5.17:18115/real","status":403,"finished":true,"startedAtMs":43},
            {"requestId":"refused","url":"http://10.55.5.17:18115/refused","status":403,"finished":true,"startedAtMs":44}
        ]});
        let receipt = super::network_receipt(
            "network.list",
            &snapshot,
            &snapshot,
            &[json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"})],
            1,
        );
        let rows = &receipt["untrustedPageContent"]["requests"];
        assert!(rows[0].get("refusedBy").is_none(), "{receipt}");
        assert!(rows[1].get("refusedBy").is_none(), "{receipt}");
        assert!(
            receipt["networkRequestRefusals"][0]
                .get("requestId")
                .is_none(),
            "{receipt}"
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["reason"],
            "sender unknown"
        );
    }

    #[test]
    fn a_filtered_result_cannot_hide_another_eligible_row() {
        let shown = json!({"requests":[
            {"requestId":"server-403","url":"http://10.55.5.17:18115/real","status":403}
        ]});
        let snapshot = json!({"requests":[
            {"requestId":"server-403","url":"http://10.55.5.17:18115/real","status":403,"startedAtMs":43},
            {"requestId":"refused","url":"http://10.55.5.17:18115/refused","error":"request failed","startedAtMs":44}
        ],"dropped":0});
        let receipt = super::network_receipt(
            "network.list",
            &shown,
            &snapshot,
            &[json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"})],
            1,
        );
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
        assert!(
            receipt["networkRequestRefusals"][0]
                .get("requestId")
                .is_none(),
            "{receipt}"
        );
    }

    #[test]
    fn an_incomplete_native_log_keeps_the_refusal_separate() {
        let snapshot = json!({"requests":[
            {"requestId":"server-403","url":"http://10.55.5.17:18115/real","status":403}
        ],"dropped":1});
        let receipt = super::network_receipt(
            "network.list",
            &snapshot,
            &snapshot,
            &[json!({"atMs":42,"target":"10.55.5.17:18115","reason":"sender unknown"})],
            1,
        );
        assert!(
            receipt["untrustedPageContent"]["requests"][0]
                .get("refusedBy")
                .is_none(),
            "{receipt}"
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["reason"],
            "sender unknown"
        );
    }

    #[test]
    fn a_single_native_error_row_can_name_its_refusal() {
        let snapshot = json!({"requests":[
            {"requestId":"native-error","url":"http://10.55.5.17:18115/refused","error":"The network connection was lost.","startedAtMs":43}
        ],"dropped":0});
        let receipt = super::network_receipt(
            "network.list",
            &snapshot,
            &snapshot,
            &[json!({"atMs":42,"target":"10.55.5.17:18115","reason":"question not answered"})],
            1,
        );
        assert_eq!(
            receipt["untrustedPageContent"]["requests"][0]["refusedBy"],
            "Vibe Work"
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["requestId"],
            "native-error"
        );
    }

    #[test]
    fn network_list_keeps_host_block_separate_from_native_failed_fetch() {
        let snapshot = json!({"requests":[{"error":"net::ERR_FAILED"}]});
        let receipt = super::network_receipt(
            "network.list",
            &snapshot,
            &snapshot,
            &[json!({
                "atMs": 42,
                "origin": "http://example.com",
                "target": "10.56.4.13:18115",
                "targetClass": "local-network",
                "reason": "blocked by the user",
                "message": "Browser request refused by Vibe Work: example.com is blocked from reaching apps and devices on this computer or your local network"
            })],
            1,
        );
        assert_eq!(
            receipt["untrustedPageContent"]["requests"][0]["error"],
            "net::ERR_FAILED"
        );
        assert_eq!(
            receipt["networkRequestRefusals"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["reason"],
            "blocked by the user"
        );
        assert_eq!(
            receipt["networkRequestRefusals"][0]["targetClass"],
            "local-network"
        );
    }
}
