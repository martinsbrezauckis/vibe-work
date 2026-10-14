//! The `ui/*` route registry: later blocks (on) register UI-control hooks (select, click, read)
//! here without editing this crate's router. Each hook takes a JSON body and returns JSON.
//!
//! Depends on: vw-core-api (`CoreError`). Called by: UI blocks (register), `routes` (`GET /ui`,
//! `POST /ui/{name}`).
//! Invariants:
//! - A hook runs behind the same guard as every route (Host, Origin, token).
//! - A hook that needs Vibe must go through `Core::execute` like every other route: the registry
//!   hands it nothing that reaches the transport.
//! - Names are lowercase `[a-z0-9-]` segments joined by `/`, at most 64 characters, registered
//!   once; a duplicate or malformed name is refused.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use serde_json::Value;
use vw_core_api::CoreError;

type Answer = Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>>;

/// A registered hook.
pub(crate) type UiHandler = Arc<dyn Fn(Value) -> Answer + Send + Sync>;

struct UiRoute {
    description: String,
    handler: UiHandler,
}

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The name does not follow the naming rule.
    #[error(
        "invalid ui route name {0:?}: lowercase [a-z0-9-] segments joined by '/', up to 64 characters"
    )]
    InvalidName(String),
    /// The name is taken.
    #[error("ui route {0:?} is already registered")]
    Duplicate(String),
}

/// Shared table of UI routes. Clone it into every block that registers hooks.
#[derive(Clone, Default)]
pub struct UiRoutes(Arc<RwLock<BTreeMap<String, UiRoute>>>);

fn valid_name(name: &str) -> bool {
    name.len() <= 64
        && !name.is_empty()
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

impl UiRoutes {
    /// Registers `handler` at `POST /ui/<name>`.
    ///
    /// # Errors
    /// [`RegistryError`] for a malformed or duplicate name.
    pub fn register<F, Fut>(
        &self,
        name: &str,
        description: &str,
        handler: F,
    ) -> Result<(), RegistryError>
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, CoreError>> + Send + 'static,
    {
        if !valid_name(name) {
            return Err(RegistryError::InvalidName(name.chars().take(80).collect()));
        }
        let mut routes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if routes.contains_key(name) {
            return Err(RegistryError::Duplicate(name.to_owned()));
        }
        let handler: UiHandler = Arc::new(move |body| Box::pin(handler(body)));
        routes.insert(
            name.to_owned(),
            UiRoute {
                description: description.to_owned(),
                handler,
            },
        );
        Ok(())
    }

    /// Registered names with their descriptions, sorted by name.
    #[must_use]
    pub fn list(&self) -> Vec<(String, String)> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(name, route)| (name.clone(), route.description.clone()))
            .collect()
    }

    pub(crate) fn get(&self, name: &str) -> Option<UiHandler> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .map(|route| route.handler.clone())
    }
}
