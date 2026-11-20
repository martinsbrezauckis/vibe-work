//! Converts `WKWebView` request events into the shared bounded log. Native callbacks belong to one webview.
//! Keeps native-to-tool ids only for active requests; top-document navigation discards previous observations.
use super::{Shared, lock};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use vw_platform::webview::content_rule_event::ContentRuleAction;
use vw_platform::webview::request_event::{RequestEvent, RequestStep};

#[derive(Default)]
struct Requests {
    ids: HashMap<u64, String>,
    retired: VecDeque<u64>,
}
impl Requests {
    fn record(&mut self, log: &Shared, event: &RequestEvent) {
        if self.retired.contains(&event.id) {
            return;
        }
        let cached = event.step == RequestStep::Completed
            && event.from_cache
            && !self.ids.contains_key(&event.id);
        if event.main_document && (event.step == RequestStep::Sent || cached) {
            lock(log).reset();
            self.retired.extend(self.ids.keys().copied());
            while self.retired.len() > 512 {
                self.retired.pop_front();
            }
            self.ids.clear();
        }
        if event.step == RequestStep::Redirected {
            if let Some(id) = self.ids.remove(&event.id) {
                let mut fields = response(event);
                fields["finished"] = json!(true);
                lock(log).update(&id, fields);
            } else {
                return;
            }
        }
        if matches!(event.step, RequestStep::Sent | RequestStep::Redirected) || cached {
            if self.ids.len() >= 512 {
                self.ids.clear();
            }
            let mut row = json!({"url":event.url,"method":event.method,"type":event.resource_type,"finished":false});
            if let Some(headers) = &event.request_headers {
                row["requestHeaders"] = json!(headers.values);
                row["requestHeadersTruncated"] = json!(headers.truncated);
            }
            let id = lock(log).start(row);
            self.ids.insert(event.id, id);
        }
        if matches!(event.step, RequestStep::Response | RequestStep::Completed)
            && let Some(id) = self.ids.get(&event.id)
        {
            let mut fields = response(event);
            if event.step == RequestStep::Completed {
                fields["finished"] = json!(true);
                if let Some(error) = &event.error {
                    fields["error"] = json!(error.chars().take(500).collect::<String>());
                }
            }
            lock(log).update(id, fields);
            if event.step == RequestStep::Completed {
                self.ids.remove(&event.id);
            }
        }
    }
}

/// The own-address refusal in a content-rule action of webview `label`: a load that `label`'s own network-policy list
/// (its identifier is the label) stopped, to this Mac's non-loopback interface address. Other lists (Browser shields)
/// and other targets are not this refusal.
fn own_address_block(
    action: &ContentRuleAction,
    label: &str,
    own: &[std::net::IpAddr],
) -> Option<(String, u16)> {
    if !action.blocked_load || action.list != label {
        return None;
    }
    let url = url::Url::parse(&action.url).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return None;
    }
    let ip = match url.host()? {
        url::Host::Ipv4(ip) => std::net::IpAddr::V4(ip),
        url::Host::Ipv6(ip) => std::net::IpAddr::V6(ip),
        url::Host::Domain(_) => return None,
    }
    .to_canonical();
    if ip.is_loopback() || !own.contains(&ip) {
        return None;
    }
    Some((ip.to_string(), url.port_or_known_default()?))
}

fn response(event: &RequestEvent) -> Value {
    let mut fields = json!({"fromCache":event.from_cache});
    if event.status > 0 {
        fields["status"] = json!(event.status);
        fields["finalUrl"] = json!(event.response_url.as_ref().unwrap_or(&event.url));
    }
    if let Some(headers) = &event.response_headers {
        fields["responseHeaders"] = json!(headers.values);
        fields["responseHeadersTruncated"] = json!(headers.truncated);
    }
    fields
}

#[cfg(target_os = "macos")]
pub(super) fn install(
    view: *mut std::ffi::c_void,
    log: Shared,
    label: &str,
    hooks: std::sync::Arc<dyn vw_preview::PreviewHooks>,
) -> Result<(), String> {
    use std::sync::{Arc, Mutex, PoisonError};
    // A content-rule block never starts a resource load, so the request watch below never sees it; WebKit reports it
    // to the navigation delegate instead (`content_rule_event`).
    let rule_label = label.to_owned();
    vw_platform::webview::content_rule_event::watch(
        label,
        Arc::new(move |action| {
            if let Some((host, port)) =
                own_address_block(&action, &rule_label, &vw_netguard::own::own_addresses())
            {
                hooks.content_rule_refused_own_address(&rule_label, &host, port);
            }
        }),
    );
    let requests = Mutex::new(Requests::default());
    let attached = vw_platform::webview::request_watch_mac::attach(
        view,
        Arc::new(move |event| {
            let mut requests = requests.lock().unwrap_or_else(PoisonError::into_inner);
            requests.record(&log, &event);
        }),
    );
    if attached {
        Ok(())
    } else {
        Err("macOS did not expose its request observation interface. Update macOS and restart Vibe Work, then retry network.list; inspect the page in developer tools if it still fails.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use vw_platform::webview::request_event::HeaderFields;

    fn log() -> Shared {
        Arc::new(Mutex::new(super::super::Log::default()))
    }
    fn event(id: u64, step: RequestStep) -> RequestEvent {
        RequestEvent {
            id,
            step,
            url: format!("https://example.test/{id}"),
            response_url: None,
            method: "GET".into(),
            resource_type: 5,
            status: 0,
            from_cache: false,
            main_document: false,
            request_headers: None,
            response_headers: None,
            error: None,
        }
    }
    #[test]
    fn mac_requests_keep_real_headers_status_errors_and_redirects_in_their_own_view() {
        let (a, b) = (log(), log());
        let mut watch = Requests::default();
        let mut sent = event(1, RequestStep::Sent);
        sent.request_headers = Some(HeaderFields {
            values: [("Accept".into(), "application/json".into())].into(),
            truncated: false,
        });
        watch.record(&a, &sent);
        let mut redirect = event(1, RequestStep::Redirected);
        redirect.url = "https://example.test/final".into();
        redirect.response_url = Some("https://example.test/1".into());
        redirect.status = 302;
        watch.record(&a, &redirect);
        let mut response = event(1, RequestStep::Response);
        response.url = "https://example.test/final".into();
        response.status = 503;
        response.response_headers = Some(HeaderFields {
            values: [("Content-Type".into(), "application/json".into())].into(),
            truncated: true,
        });
        watch.record(&a, &response);
        let mut done = event(1, RequestStep::Completed);
        done.status = 503;
        done.error = Some("connection closed".into());
        watch.record(&a, &done);
        let rows = lock(&a).snapshot().unwrap();
        assert_eq!(
            rows["requests"][0]["requestHeaders"]["Accept"],
            "application/json"
        );
        assert_eq!(rows["requests"][0]["status"], 302);
        assert_eq!(rows["requests"][0]["finalUrl"], "https://example.test/1");
        assert_eq!(rows["requests"][1]["status"], 503);
        assert_eq!(
            rows["requests"][1]["responseHeaders"]["Content-Type"],
            "application/json"
        );
        assert_eq!(rows["requests"][1]["responseHeadersTruncated"], true);
        assert_eq!(rows["requests"][1]["error"], "connection closed");
        assert_eq!(rows["requests"][1]["finished"], true);
        assert_eq!(lock(&b).snapshot().unwrap()["requests"], json!([]));
        assert!(watch.ids.is_empty());
    }
    #[test]
    fn mac_navigation_resets_rows_and_late_old_callbacks_do_not_reappear() {
        let log = log();
        let mut watch = Requests::default();
        watch.record(&log, &event(1, RequestStep::Sent));
        let mut nav = event(2, RequestStep::Sent);
        nav.main_document = true;
        watch.record(&log, &nav);
        let mut late = event(1, RequestStep::Completed);
        late.from_cache = true;
        late.main_document = true;
        watch.record(&log, &late);
        assert_eq!(
            lock(&log).snapshot().unwrap()["requests"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            lock(&log).snapshot().unwrap()["requests"][0]["url"],
            "https://example.test/2"
        );
    }
    #[test]
    fn mac_cached_completions_without_a_send_are_observed_and_logs_are_bounded() {
        let log = log();
        let mut watch = Requests::default();
        let mut cached = event(1, RequestStep::Completed);
        cached.from_cache = true;
        cached.status = 200;
        watch.record(&log, &cached);
        assert_eq!(
            lock(&log).snapshot().unwrap()["requests"][0]["fromCache"],
            true
        );
        for id in 2..900 {
            watch.record(&log, &event(id, RequestStep::Sent));
        }
        let snapshot = lock(&log).snapshot().unwrap();
        assert_eq!(snapshot["requests"].as_array().unwrap().len(), 256);
        assert!(snapshot["dropped"].as_u64().unwrap() > 0);
        assert!(watch.ids.len() <= 512);
    }

    /// PREDICT (written before the change): `own_address_block` did not exist (compile red); the old classifier read
    /// request-watch completions, which a content-rule block never produces.
    #[test]
    fn own_list_block_of_this_macs_address_is_the_own_address_refusal() {
        let own = ["10.55.5.19".parse().unwrap(), "fd00::5".parse().unwrap()];
        let action = |list: &str, url: &str, blocked_load: bool| ContentRuleAction {
            list: list.into(),
            url: url.into(),
            blocked_load,
        };
        let label = "preview-7";
        let check = |a: ContentRuleAction| own_address_block(&a, label, &own);
        assert_eq!(
            check(action(
                label,
                "http://10.55.5.19:18115/p.txt?secret=1",
                true
            )),
            Some(("10.55.5.19".to_owned(), 18115))
        );
        assert_eq!(
            check(action(label, "https://10.55.5.19/", true)),
            Some(("10.55.5.19".to_owned(), 443))
        );
        assert_eq!(
            check(action(label, "http://[fd00::5]:81/", true)),
            Some(("fd00::5".to_owned(), 81))
        );
        assert_eq!(
            check(action(label, "http://[::ffff:10.55.5.19]:81/", true)),
            Some(("10.55.5.19".to_owned(), 81))
        );
        // Not this refusal: another list, no block, loopback, another address, a name, another scheme.
        assert_eq!(
            check(action("shields", "http://10.55.5.19:81/", true)),
            None
        );
        assert_eq!(check(action(label, "http://10.55.5.19:81/", false)), None);
        assert_eq!(check(action(label, "http://127.0.0.1:81/", true)), None);
        assert_eq!(check(action(label, "http://10.55.5.20:81/", true)), None);
        assert_eq!(check(action(label, "http://host.local:81/", true)), None);
        assert_eq!(check(action(label, "ftp://10.55.5.19/", true)), None);
        assert_eq!(check(action(label, "not a url", true)), None);
    }
}
