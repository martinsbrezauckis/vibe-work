//! Marketplace fixture input for app startup; depends only on std and is called by startup.
//! Ordinary builds keep the shipped catalog. The test feature is refused in release builds.

use std::path::PathBuf;

#[cfg(all(feature = "vault-e2e", not(debug_assertions)))]
compile_error!(
    "the vault-e2e feature is for debug test builds only; it must never be in a release build"
);

pub(super) fn index() -> Option<PathBuf> {
    #[cfg(feature = "vault-e2e")]
    {
        std::env::var_os("VW_E2E_MARKET_INDEX").map(PathBuf::from)
    }
    #[cfg(not(feature = "vault-e2e"))]
    {
        None
    }
}
