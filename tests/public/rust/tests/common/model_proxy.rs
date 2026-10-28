//! Routes scripted-model tests through the product key proxy. Called by real-Vibe rigs;
//! only the proxy owns the upstream fixture key, and Vibe receives a rotating placeholder.
use std::sync::Arc;

use vw_keyproxy::{KeyCell, KeyState, ProxyConfig, ProxyHandle, ProxySecretProvider, TokenSlot};
use vw_supervisor::SupervisorConfig;

use super::ScriptedModel;

impl ScriptedModel {
    pub async fn route(&self, server: &mut SupervisorConfig) -> Arc<ProxySecretProvider> {
        self.route_with(server, ProxyConfig::default()).await
    }

    pub async fn route_with(
        &self,
        server: &mut SupervisorConfig,
        mut config: ProxyConfig,
    ) -> Arc<ProxySecretProvider> {
        let key = "sk-vw-canary-7f3a9c1e5b2d4086";
        vw_secrets::register(vw_secrets::SecretKind::MistralKey, key);
        let keys = KeyCell::new(KeyState::Ready(Arc::new(key.to_owned().into())));
        let tokens = Arc::new(TokenSlot::default());
        config.upstreams = vw_keyproxy::Upstreams {
            api: self.server().to_owned(),
            console: format!("{}/console", self.server()),
            chat: format!("{}/chat", self.server()),
        };
        let proxy = ProxyHandle::start(config, tokens.clone(), keys)
            .await
            .unwrap();
        // Replace the complete provider environment, exactly as KeyService does in the app.
        let env = vw_keys::vibe_env(&proxy.base_url());
        server
            .extra_env
            .retain(|(key, _)| !env.iter().any(|(name, _)| name == key));
        server.extra_env.extend(env);
        let mut state = self.inner.lock().unwrap();
        state.expected_key = Some(key.to_owned());
        state.proxies.push(proxy);
        Arc::new(ProxySecretProvider::new(tokens))
    }
}
