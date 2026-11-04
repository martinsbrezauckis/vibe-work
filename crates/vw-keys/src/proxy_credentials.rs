//! Proxy sign-ins as user-only vault resources. Called by network Save, launch and migration.
//! Public settings hold only endpoints and opaque item ids; never trust an id without its record's MAC.

use crate::{KeyError, service::now_ms};
use base64::Engine as _;
use std::collections::BTreeMap;
use vw_mistral::network::{NetworkSettings, has_sign_in, mask_proxy_url};
use vw_vault::items::{ResourceKind, ResourcePermission, VaultResource};
use vw_vault::{SecretString, Vault};

pub(crate) const PROXIES: [&str; 3] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"];
const FIELD: &str = "proxyUrl";

pub(crate) fn protected(settings: &NetworkSettings) -> bool {
    !settings.proxy_credentials.is_empty()
        || PROXIES
            .iter()
            .any(|key| settings.get(key).is_some_and(|v| has_sign_in(&v)))
}

/// Store complete credentials, including their endpoint binding, only inside the existing encrypted vault.
pub(crate) fn store(
    vault: &Vault,
    settings: &NetworkSettings,
) -> Result<NetworkSettings, KeyError> {
    let mut public = settings.clone();
    for key in PROXIES {
        let Some(raw) = settings.get(key).filter(|v| has_sign_in(v)) else {
            continue;
        };
        if raw.contains("://***@") {
            return Err(KeyError::InvalidSecret(
                "Unlock the vault to keep the hidden proxy sign-in.",
            ));
        }
        register(&raw);
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| KeyError::Internal)?;
        let id = format!("vibe-work-proxy-{}", hex::encode(random));
        let now = now_ms();
        let item = VaultResource {
            id: id.clone(),
            schema_version: vw_vault::items::RESOURCE_SCHEMA_VERSION.to_owned(),
            kind: ResourceKind::DeveloperCredential,
            label: "Proxy sign-in (Vibe Work)".to_owned(),
            permission: ResourcePermission::UserOnly,
            public_fields: BTreeMap::new(),
            secret_fields: BTreeMap::from([
                (FIELD.to_owned(), raw),
                (
                    "proxyCertificates".into(),
                    serde_json::to_string(
                        &settings
                            .guard_certificates()
                            .iter()
                            .map(|root| base64::prelude::BASE64_STANDARD.encode(root))
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|_| KeyError::Internal)?,
                ),
            ]),
            custom_fields: Vec::new(),
            created_ms: now,
            updated_ms: now,
            extra: BTreeMap::new(),
        }
        .to_item()?;
        vault
            .update_item(&id, |_| Some(item.clone()))?
            .ok_or(KeyError::Internal)?;
        public = public.with(key, settings.get(key).map(|v| mask_proxy_url(&v)));
        public.proxy_credentials.insert(key.to_owned(), id);
    }
    Ok(public)
}

/// Resolve authenticated references. Locked or missing credentials are errors, never an unauthenticated fallback.
pub(crate) fn restore(
    vault: &Vault,
    public: &NetworkSettings,
) -> Result<NetworkSettings, KeyError> {
    let mut settings = public.clone();
    for (key, id) in &public.proxy_credentials {
        if !PROXIES.contains(&key.as_str()) {
            return Err(KeyError::Internal);
        }
        let item = vault.read_item(id)?.ok_or(KeyError::Internal)?;
        let resource = VaultResource::from_item(&item).ok_or(KeyError::Internal)?;
        if resource.permission != ResourcePermission::UserOnly
            || resource.kind != ResourceKind::DeveloperCredential
        {
            return Err(KeyError::Internal);
        }
        let raw = SecretString::new(
            resource
                .secret_fields
                .get(FIELD)
                .ok_or(KeyError::Internal)?
                .clone(),
        );
        register(raw.expose());
        if Some(mask_proxy_url(raw.expose())) != public.get(key) {
            return Err(KeyError::Internal);
        }
        settings = settings.with(key, Some(raw.expose().to_owned()));
        if let Some(encoded) = resource.secret_fields.get("proxyCertificates") {
            let roots: Vec<String> =
                serde_json::from_str(encoded).map_err(|_| KeyError::Internal)?;
            let roots = roots
                .iter()
                .map(|root| {
                    base64::prelude::BASE64_STANDARD
                        .decode(root)
                        .map_err(|_| KeyError::Internal)
                })
                .collect::<Result<Vec<_>, _>>()?;
            settings.proxy_certificates.insert(key.clone(), roots);
        }
    }
    settings
        .proxy_credentials
        .clone_from(&public.proxy_credentials);
    Ok(settings)
}

/// Register the URL and userinfo before a confirmed stored credential can enter diagnostics.
pub(crate) fn register(raw: &str) {
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, raw);
    if let Ok(url) = url::Url::parse(raw) {
        vw_secrets::register(vw_secrets::SecretKind::VaultValue, url.username());
        if let Some(password) = url.password() {
            vw_secrets::register(vw_secrets::SecretKind::VaultValue, password);
        }
    }
}

/// A locked Settings view retains the existing hidden-sign-in marker, never a live credential.
pub(crate) fn masked(public: &NetworkSettings) -> NetworkSettings {
    let mut settings = public.clone();
    for key in public.proxy_credentials.keys() {
        if let Some(raw) = public.get(key) {
            settings = settings.with(key, Some(raw.replacen("://", "://***@", 1)));
        }
    }
    settings
        .proxy_credentials
        .clone_from(&public.proxy_credentials);
    settings
}
