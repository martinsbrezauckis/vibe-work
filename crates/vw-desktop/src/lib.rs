//! Standalone Linux accessibility pilot. Uses typed AT-SPI2 proxies; called by Rust clients and tests.
//! No app, agent tool or UI integration. Every action is followed by a fresh postcondition read.

#![cfg(target_os = "linux")]

mod backend;
mod engine;
mod model;

pub use atspi::{Role, State, StateSet};
pub use model::*;

use backend::Atspi;
use engine::Engine;

/// A connection to the current user's Linux accessibility bus.
pub struct Desktop {
    engine: Engine<Atspi>,
}

impl Desktop {
    /// Connect through `org.a11y.Bus` on the session D-Bus.
    ///
    /// # Errors
    /// Returns the connection error if the session or accessibility bus is unavailable.
    pub async fn connect() -> Result<Self, Error> {
        Ok(Self {
            engine: Engine(Atspi::connect().await?),
        })
    }

    /// Connect to an explicit accessibility bus address supplied by the caller.
    /// Useful when a host already knows the bus address, or for an isolated test bus.
    ///
    /// # Errors
    /// Returns malformed D-Bus addresses or connection errors.
    pub async fn connect_at(address: &str) -> Result<Self, Error> {
        Ok(Self {
            engine: Engine(Atspi::connect_at(address).await?),
        })
    }

    /// Read the active window, or the window with this exact accessible name.
    ///
    /// # Errors
    /// Returns bus errors, missing/ambiguous window errors, or incomplete window discovery.
    pub async fn snapshot(&self, window: Option<&str>) -> Result<Snapshot, Error> {
        self.engine.snapshot(window).await
    }

    /// Perform one mutation and verify its postcondition with fresh bus reads.
    /// A returned receipt is successful only when [`Receipt::is_success`] is true.
    ///
    /// # Errors
    /// Returns invalid requests, unavailable interfaces/elements, or bus errors. A transport error
    /// after dispatch does not prove the action failed to run; inspect before any manual retry.
    pub async fn act(&self, element: &ElementId, action: Action) -> Result<Receipt, Error> {
        self.engine.act(element, action).await
    }

    /// Re-read this element and compare it to the requested observable state.
    /// This is read-only and never repeats an action.
    ///
    /// # Errors
    /// Returns invalid postconditions, unavailable interfaces/elements, or bus errors.
    pub async fn verify(
        &self,
        element: &ElementId,
        expected: &Postcondition,
    ) -> Result<Verification, Error> {
        self.engine.verify(element, expected).await
    }
}

#[cfg(test)]
mod tests;
