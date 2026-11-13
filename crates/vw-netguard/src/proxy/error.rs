//! Proxy failure states and agent instructions, shared by discovery and transports. Never hold remote text.

/// A proxy failure with fixed, credential-free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Invalid configuration, unsupported protocol, or malformed PAC result.
    Configuration,
    /// System proxy/PAC discovery failed.
    Discovery,
    /// Proxy could not be reached.
    Unreachable,
    /// A bounded operation timed out.
    Timeout,
    /// TLS peer or trust configuration failed verification.
    Certificate,
    /// Proxy rejected credentials or requested an unsupported authentication method.
    Authentication,
    /// Proxy refused the checked destination.
    Refused,
    /// Proxy returned malformed protocol bytes.
    Protocol,
}
impl Error {
    /// The short state shown to the person.
    #[must_use]
    pub fn state(self) -> &'static str {
        match self {
            Self::Configuration => "The proxy settings are not valid.",
            Self::Discovery => "The system proxy could not be resolved.",
            Self::Unreachable => "The proxy could not be reached.",
            Self::Timeout => "The proxy did not answer in time.",
            Self::Certificate => "The proxy certificate could not be verified.",
            Self::Authentication => "The proxy sign-in was refused.",
            Self::Refused => "The proxy refused the connection.",
            Self::Protocol => "The proxy response was not valid.",
        }
    }
    /// Diagnosis and instruction recorded for the agent. No hostname retry can bypass the guard.
    #[must_use]
    pub fn outcome(self) -> &'static str {
        match self {
            Self::Configuration => {
                "The configured proxy or PAC result is invalid or unsupported. Ask the user to check Settings > Network or the system proxy. Do not bypass the guard."
            }
            Self::Discovery => {
                "The OS proxy/PAC resolver failed or is unavailable. Ask the user to check the system proxy; on Linux install libproxy with the desktop/PAC backend or set a manual proxy in Settings > Network. Do not retry directly."
            }
            Self::Unreachable => {
                "The configured proxy could not be reached. Ask the user to check their network and proxy address, then retry. Do not bypass the proxy."
            }
            Self::Timeout => {
                "The proxy connection or system PAC lookup timed out. Ask the user to check their proxy, then retry. Do not bypass the proxy."
            }
            Self::Certificate => {
                "TLS verification of the proxy failed. Ask the user to check the proxy name and company CA in Settings > Network. Never disable certificate verification."
            }
            Self::Authentication => {
                "The proxy rejected sign-in or requested an unsupported authentication method. Ask the user to check the proxy credentials in Settings or the system configuration. Never put credentials in chat or logs."
            }
            Self::Refused => {
                "The proxy refused the guard's checked IP destination. Ask the user to check the proxy's access rules. Never retry by hostname or bypass the guard."
            }
            Self::Protocol => {
                "The proxy returned an invalid handshake. Ask the user to check the proxy type and address, then retry. Do not bypass the guard."
            }
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.state())
    }
}
impl std::error::Error for Error {}
