//! Standalone Vibe skills requests and UI projection. Depends only on serde and
//! typed errors; future app adapters supply transport without workspace coupling.

mod client;
mod error;
mod view;
mod wire;

pub use client::{SkillsClient, SkillsTransport};
pub use error::{SkillsError, TransportError};
pub use view::{SkillView, SkillsSource, SkillsView};
pub use wire::*;
