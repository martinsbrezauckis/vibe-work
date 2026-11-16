//! Picked folders and open callbacks on the gate: registering a natively picked folder, and the callback
//! book (register, validate and send an answer, close by callback or session).
use std::path::Path;

use serde_json::{Value, json};
use tracing::warn;

use crate::callbacks::{CallbackKind, OpenCallback};
use crate::confirm::Confirmer;
use crate::error::{CallbackFault, GateError};
use crate::transport::Transport;

use super::{Gate, log_refusal};

impl<T: Transport, C: Confirmer> Gate<T, C> {
    /// Records a folder the user picked in a native dialog. Only such folders (and folders inside
    /// them) can be trusted through the gate.
    ///
    /// # Errors
    /// [`GateError::InvalidFolder`] when the path does not exist or is not a directory.
    pub fn register_picked_folder(&self, path: &Path) -> Result<(), GateError> {
        self.state().picked.register(path)
    }

    /// True when `path` is a folder the user picked in a native dialog in this run, or lies inside one
    /// (both resolved, so `..` hops and symlinks out of a picked folder do not count). A relative or
    /// unresolvable path is never picked. The `worktrees` command acts only on such folders
    /// unless the user confirms in the native dialog.
    #[must_use]
    pub fn is_picked_folder(&self, path: &Path) -> bool {
        self.state().picked.contains_path(path)
    }

    /// The folders the user picked in a native dialog in this run (canonical paths). The file
    /// resolver compares a candidate with these LEXICALLY before any filesystem access (no oracle for paths
    /// outside them), then checks the canonical path again.
    #[must_use]
    pub fn picked_roots(&self) -> Vec<std::path::PathBuf> {
        self.state().picked.roots()
    }

    /// The picked folders whose spelling as picked (the folder dialog's, the Debug API's) differs from their canonical
    /// path, as `(spelling, canonical)`: macOS `/tmp/x` for `/private/tmp/x`, a folder reached through a symbolic link.
    /// For the same lexical comparison as [`Self::picked_roots`] in the spelling the window keeps; never for trust.
    #[must_use]
    pub fn picked_spellings(&self) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
        self.state().picked.spellings()
    }

    /// Registers the callback of an inbound `callback/call` (pass its `params`). The UI should be
    /// shown every registered callback, including kinds the gate will not answer.
    ///
    /// # Errors
    /// [`CallbackFault::InvalidRegistration`] when `params` is not a callback entry;
    /// [`CallbackFault::TooManyOpen`] past the configured limit;
    /// [`CallbackFault::DuplicateId`] for an id that was already answered.
    pub fn register_callback(&self, params: &Value) -> Result<CallbackKind, CallbackFault> {
        self.state().callbacks.register(params)
    }

    /// Checks an original answer before the core replaces an approval with a refusal. Does not claim it.
    ///
    /// # Errors
    /// [`GateError::Callback`] when the session, callback kind or offered output shape does not match.
    pub fn validate_callback_answer(
        &self,
        session_id: &str,
        callback_id: &str,
        output: &Value,
    ) -> Result<(), GateError> {
        self.state()
            .callbacks
            .validate(session_id, callback_id, output)
            .map_err(|fault| {
                let error = GateError::Callback(fault);
                log_refusal("answer_callback", "callback/result", &error);
                error
            })
    }

    /// Answers an open callback with `output` (the `output` object of `callback/result`).
    ///
    /// The answer is sent only if it matches the callback's session and kind and has exactly the
    /// output shape for that kind. The callback is claimed before the send is awaited,
    /// so concurrent answers send once. A claimed callback is never answered again, even if the
    /// send fails.
    ///
    /// # Errors
    /// [`GateError::Callback`] with the reason when the answer is refused (nothing was sent);
    /// [`GateError::Transport`] when the send fails.
    pub async fn answer_callback(
        &self,
        session_id: &str,
        callback_id: &str,
        output: Value,
    ) -> Result<Value, GateError> {
        if let Err(fault) = self
            .state()
            .callbacks
            .claim(session_id, callback_id, &output)
        {
            let error = GateError::Callback(fault);
            log_refusal("answer_callback", "callback/result", &error);
            return Err(error);
        }
        let params = json!({
            "sessionId": session_id,
            "result": {"callbackId": callback_id, "output": output},
        });
        let sent = self.transport.request("callback/result", params).await;
        self.state().callbacks.sent(session_id);
        match sent {
            Ok(result) => Ok(result),
            Err(error) => {
                warn!(
                    callback_id,
                    "gate: callback answer was claimed but the send failed"
                );
                Err(error.into())
            }
        }
    }

    /// Forgets a callback the server closed (cancelled, expired, or its turn ended).
    pub fn callback_closed(&self, callback_id: &str) {
        self.state().callbacks.close(callback_id);
    }

    /// Forgets every callback of a session (the session ended or the server restarted).
    pub fn session_closed(&self, session_id: &str) {
        self.state().callbacks.close_session(session_id);
    }

    /// The callbacks waiting for an answer, for the UI to show.
    #[must_use]
    pub fn open_callbacks(&self) -> Vec<OpenCallback> {
        self.state().callbacks.open()
    }

    /// True while an answer of this session is still being delivered.
    pub fn answering(&self, session_id: &str) -> bool {
        self.state().callbacks.sending(session_id)
    }
}
