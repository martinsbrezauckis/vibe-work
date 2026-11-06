//! Proxy facts for model input and `session_status`. Uses the authenticated Network view and the
//! process tool's launch environment; contains no credentials, vault references or behaviour instructions.
use std::{ffi::OsString, sync::Arc};

use serde_json::{Value, json};
use vw_mistral::network::mask_proxy_url;

use crate::{Core, NetworkFieldView, NetworkResult, NetworkView, core::Shared};

pub(crate) async fn read(shared: &Arc<Shared>) -> Value {
    let core = Core {
        shared: shared.clone(),
    };
    let NetworkResult::Settings { view, .. } = core.network_read().await else {
        return Value::Null;
    };
    facts(&view, &shared.app_context.env)
}

fn facts(view: &NetworkView, env: &[(OsString, OsString)]) -> Value {
    let effective = |field: &NetworkFieldView| {
        field
            .environment_value
            .as_deref()
            .unwrap_or(&field.value)
            .to_owned()
    };
    let process: serde_json::Map<String, Value> =
        ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"]
            .into_iter()
            .filter_map(|key| {
                let get = |name: &str| {
                    env.iter()
                        .rev()
                        .find(|(k, _)| k == name)
                        .and_then(|(_, v)| v.to_str())
                        .filter(|v| !v.is_empty())
                };
                let value = get(&key.to_ascii_lowercase()).or_else(|| get(key))?;
                Some((
                    key.to_owned(),
                    json!(if key == "NO_PROXY" {
                        value.to_owned()
                    } else {
                        mask_proxy_url(value)
                    }),
                ))
            })
            .collect();
    json!({
        "vibeShell": {
            "httpProxy": effective(&view.http_proxy),
            "httpsProxy": effective(&view.https_proxy),
            "allProxy": effective(&view.all_proxy),
            "noProxy": effective(&view.no_proxy),
            "alwaysDirect": view.always_direct,
            "restartNeeded": view.restart_needed,
            "applies": "Saved proxy settings apply to Vibe's shell commands after Vibe Work restarts. A running shell keeps its startup environment. Authenticated proxies use a local relay without the sign-in in the shell environment."
        },
        "processStart": {
            "proxyEnvironment": process,
            "applies": "process_start uses this separate environment, inherited when Vibe Work started; saved Network settings do not change it. An empty map means no proxy environment variables are set for these commands."
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_distinguish_saved_shell_routes_from_process_environment() {
        let view = NetworkView {
            http_proxy: NetworkFieldView {
                value: "http://***@saved.example:8080".into(),
                ..Default::default()
            },
            https_proxy: NetworkFieldView {
                value: "http://saved.example:8080".into(),
                environment_value: Some("http://env.example:8080".into()),
                ..Default::default()
            },
            all_proxy: NetworkFieldView::default(),
            no_proxy: NetworkFieldView::default(),
            certificate_file: NetworkFieldView::default(),
            certificate_folder: NetworkFieldView::default(),
            always_direct: vec!["localhost".into()],
            restart_needed: true,
            certificate_count: None,
        };
        let fact = facts(
            &view,
            &[(
                "HTTPS_PROXY".into(),
                "http://person:secret@commands.example:3128".into(),
            )],
        );
        assert_eq!(
            fact["vibeShell"]["httpProxy"],
            "http://***@saved.example:8080"
        );
        assert_eq!(fact["vibeShell"]["httpsProxy"], "http://env.example:8080");
        assert_eq!(fact["vibeShell"]["restartNeeded"], true);
        assert!(
            fact["vibeShell"]["applies"]
                .as_str()
                .unwrap()
                .contains("after Vibe Work restarts")
        );
        assert!(!fact.to_string().contains("secret"));
        assert!(!fact.to_string().contains("person"));
        assert!(
            fact["processStart"]["proxyEnvironment"]["HTTPS_PROXY"]
                .as_str()
                .unwrap()
                .contains("commands.example")
        );
        assert_eq!(
            facts(&view, &[])["processStart"]["proxyEnvironment"],
            json!({})
        );
    }
}
