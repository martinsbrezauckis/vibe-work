//! Adapter failures and plain skills errors. Depends on the generated protocol
//! error model; `SkillsClient` maps replies by code without parsing reason strings.

use crate::{ProtocolError, ProtocolErrorCode};

/// Failure supplied by the app's future channel adapter.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Vibe returned an error reply, including its original code and details.
    #[error("{}", .0.message)]
    Reply(ProtocolError),
    /// The channel could not deliver the request or response.
    #[error("{0}")]
    Unavailable(String),
}

/// Small error set for the future manager; server reasons remain intact.
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    /// Input or a configuration constraint was rejected by Vibe.
    #[error("{}", .0.message)]
    InvalidInput(ProtocolError),
    /// Vibe requires the current session or turn to settle first.
    #[error("{}", .0.message)]
    Busy(ProtocolError),
    /// Registry access was denied or requires authentication.
    #[error("{}", .0.message)]
    AccessDenied(ProtocolError),
    /// The connected server does not implement the requested method.
    #[error("{}", .0.message)]
    Unsupported(ProtocolError),
    /// Other server failures retain their structured error reply.
    #[error("{}", .0.message)]
    Server(ProtocolError),
    /// A channel adapter failed before a server reply arrived.
    #[error("{0}")]
    Unavailable(String),
    /// Known wire fields have the wrong type or a required field is absent.
    #[error("Could not read the skills response: {0}")]
    Wire(#[from] serde_json::Error),
}

impl From<TransportError> for SkillsError {
    fn from(error: TransportError) -> Self {
        let reply = match error {
            TransportError::Reply(reply) => reply,
            TransportError::Unavailable(message) => return Self::Unavailable(message),
        };
        match reply.code {
            ProtocolErrorCode::InvalidParams | ProtocolErrorCode::InvalidRequest => {
                Self::InvalidInput(reply)
            }
            ProtocolErrorCode::Conflict => Self::Busy(reply),
            ProtocolErrorCode::Unauthorized | ProtocolErrorCode::Forbidden => {
                Self::AccessDenied(reply)
            }
            ProtocolErrorCode::MethodNotFound | ProtocolErrorCode::NotImplemented => {
                Self::Unsupported(reply)
            }
            _ => Self::Server(reply),
        }
    }
}
