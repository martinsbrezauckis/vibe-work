//! Which host name a navigation must be looked up for, and the refusal text while that lookup runs.

use url::Url;

/// A host name a navigation or sub-request must be looked up for. `lan`: it is the host of the Lan page the user
/// opened, so a local-network answer is accepted for it ([`crate::names::NameVerdict::refusal_on`]).
pub(super) struct NameCheck {
    pub(super) host: String,
    pub(super) lan: bool,
}

/// The host name a navigation must be looked up for: a `Host::Domain` other than `localhost` and the canary suffix
/// (`None`: nothing to look up). The Lan mount's own host IS looked up (to limit DNS rebinding):
/// only its local-network answer is accepted, never loopback, link-local or special ones.
pub(super) fn name_to_check(url: &Url, lan_host: Option<&str>) -> Option<NameCheck> {
    let url::Host::Domain(name) = url.host()? else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    (lower != "localhost" && !lower.ends_with(vw_netguard::CANARY_SUFFIX)).then(|| NameCheck {
        lan: lan_host.is_some_and(|h| h.eq_ignore_ascii_case(&lower)),
        host: lower,
    })
}

/// The refusal text while a name's lookup is still running and nothing later checks it.
pub(super) fn pending_text(host: &str) -> String {
    format!("checking where {host} points; open it again in a moment")
}
