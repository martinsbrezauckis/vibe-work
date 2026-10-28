//! What one Debug API `ui/*` request lets the window run as the Debug API: the request's **grants**.
//!
//! The Debug API acts as the owner (no native dialogs, folders given as paths), except for the vault. Its `ui/*`
//! routes are carried out by the page's JavaScript, which shares its realm with anything injected into the page, so
//! the page can hold no authority: every window command runs as the window (`Origin::Ui`). The few routes that
//! promise an owner-only effect get it from here instead. When the authenticated request is parked
//! ([`crate::UiBridge::request`]) this module turns its route and body into grants: each one names one exact window
//! command (its `type`, its `op` and every security-relevant value the request names), and the host runs the first
//! window command that matches as the Debug API, once, while the request is pending
//! ([`crate::Host::execute_window`]).
//!
//! Rules, so a grant can never be wider than what the owner asked for:
//! - A grant is built only from the request's own route and body and from host-side facts ([`GrantFacts`]: the
//!   pinned catalog); never from anything the page says.
//! - Every value the request names is pinned (a path, a session, an id, a URL). A value the request leaves to the
//!   page's state (the shown session when `sessionId` is absent) is not pinned: the grant then covers that command
//!   for any such value (a residual by design: it stays within what the window may already do).
//! - Generic routes (`click`, `key`, `menu`, `palette`, `search run`) get no grant: what a click leads to is decided
//!   by page code. Their native dialogs open as the window's, and the Debug API answers them with `POST /asks/{id}`.
//! - Commands that behave the same for both origins, or that only need the Debug API's name in records, need no
//!   grant: the window runs them, and the core attributes window commands sent while a request is served to the
//!   Debug API (`vw_core_api` `debug_api_acts`).
//!
//! Depends on: `serde_json`, vw-core-api (the catalog). Called by: [`crate::bridge`] (builds and matches grants).

use serde_json::{Value, json};

/// Facts the host knows that a grant may pin (never from the page).
pub trait GrantFacts: Send + Sync {
    /// The address of catalog server `id` in the reviewed catalog.
    fn catalog_server_url(&self, id: &str) -> Option<String>;
}

/// The core's facts ([`crate::Host`] installs them on the bridge).
pub(crate) struct CoreFacts(pub(crate) vw_core_api::Core);

impl GrantFacts for CoreFacts {
    fn catalog_server_url(&self, id: &str) -> Option<String> {
        self.0.catalog_server_url(id)
    }
}

/// One window command a request lets run as the Debug API, once.
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    /// What it is for (logged when used).
    pub(crate) name: &'static str,
    /// JSON pointers into the command and the value each must have; [`Value::Null`] means "absent or null".
    pins: Vec<(&'static str, Value)>,
    /// Used already.
    pub(crate) used: bool,
}

impl Grant {
    fn new(name: &'static str, pins: Vec<(&'static str, Value)>) -> Self {
        Self {
            name,
            pins,
            used: false,
        }
    }

    /// True when the grant is unused and `command` (a window command as JSON) has every pinned value.
    #[must_use]
    pub fn matches(&self, command: &Value) -> bool {
        !self.used
            && self.pins.iter().all(|(pointer, want)| {
                let got = command.pointer(pointer).unwrap_or(&Value::Null);
                got == want
            })
    }
}

/// The body's string field `key`, if it is a non-empty string.
fn text<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// `pins` plus `(pointer, body[key])` when the body names `key` (a string): an optional value is pinned when given.
fn pin_if(
    mut pins: Vec<(&'static str, Value)>,
    pointer: &'static str,
    body: &Value,
    key: &str,
) -> Vec<(&'static str, Value)> {
    if let Some(value) = text(body, key) {
        pins.push((pointer, json!(value)));
    }
    pins
}

/// `pickFolder {path}`: the Debug API's "Choose folder" (the window is refused it).
fn pick_folder(name: &'static str, path: &str) -> Grant {
    Grant::new(
        name,
        vec![("/type", json!("pickFolder")), ("/path", json!(path))],
    )
}

fn project_folder(body: &Value) -> Vec<Grant> {
    text(body, "folder")
        .map(|path| vec![pick_folder("projects.folder", path)])
        .unwrap_or_default()
}

/// A `git` request with `op`, pinned to the body's `sessionId` and `remote` when given.
fn git(name: &'static str, op: &str, body: &Value) -> Grant {
    let pins = vec![("/type", json!("git")), ("/request/op", json!(op))];
    let pins = pin_if(pins, "/request/sessionId", body, "sessionId");
    Grant::new(name, pin_if(pins, "/request/remote", body, "remote"))
}

/// Routes that only read the page or its layout (they send no command that could act).
const PURE_ROUTES: &[&str] = &[
    "controls",
    "state",
    "revision",
    "settle",
    "rect",
    "layout",
    "trace",
    "setup",
    "secrets",
    "toasts",
    "notifications",
    "walkthrough",
];

/// Whether a request may act, so the window counts as driven by the Debug API while it is pending
/// ([`crate::UiBridge::driving`]): every request but a pure read (a read-only route, or an explicit `read` / `state`
/// action). A request without an `action` on a route that has actions counts as acting (its default may act).
#[must_use]
pub fn drives(route: &str, body: &Value) -> bool {
    !(PURE_ROUTES.contains(&route) || matches!(text(body, "action"), Some("read" | "state")))
}

/// The grants of one request: `route` is the name without `ui/`, `body` what the caller posted.
#[must_use]
pub fn for_request(route: &str, body: &Value, facts: Option<&dyn GrantFacts>) -> Vec<Grant> {
    let action = text(body, "action").unwrap_or("");
    match (route, action) {
        ("settings", "chrome") if text(body, "do") == Some("level") => text(body, "level")
            .filter(|level| matches!(*level, "browse" | "fill" | "act"))
            .map(|level| {
                vec![Grant::new(
                    "settings.chrome.level",
                    vec![
                        ("/type", json!("chromeControl")),
                        ("/request/op", json!("setLevel")),
                        ("/request/level", json!(level)),
                    ],
                )]
            })
            .unwrap_or_default(),
        // Folders are given as paths: the page's `pickFolder` for exactly this path.
        ("worktrees", "folder") => text(body, "path")
            .map(|path| vec![pick_folder("worktrees.folder", path)])
            .unwrap_or_default(),
        ("projects", "newSession" | "chooseFolder") | ("sessions", "projectStart") => {
            project_folder(body)
        }
        // A delete runs without the native confirmation and asks nothing about processes it left running.
        ("sessions", "delete") => text(body, "sessionId")
            .map(|id| {
                vec![
                    Grant::new(
                        "sessions.delete",
                        vec![
                            ("/type", json!("requestConfirmation")),
                            ("/method", json!("session/delete")),
                            ("/params", json!({ "sessionId": id })),
                        ],
                    ),
                    Grant::new(
                        "sessions.delete.leftRunning",
                        vec![
                            ("/type", json!("processes")),
                            ("/request/op", json!("afterDelete")),
                            ("/request/sessionId", json!(id)),
                        ],
                    ),
                ]
            })
            .unwrap_or_default(),
        // Push, pull request and bring-in: the core's native confirmation is answered as the Debug API's.
        ("git", "push") => vec![git("git.push", "push", body)],
        ("git", "commit") if text(body, "then") == Some("push") => {
            vec![git("git.push", "push", body)]
        }
        ("git", "createPr") => vec![git("git.createPr", "createPr", body)],
        ("git", "bringIn") => vec![git("git.bringIn", "bringIn", body)],
        ("git-browser", "click") if matches!(text(body, "target"), Some("push" | "primary")) => {
            vec![git("git-browser.push", "push", &Value::Null)]
        }
        // New chat or task here: the node the request names is a folder (a node's id is its path), chosen like
        // Choose folder (the page sends `pickFolder` for it before the copy form reads it).
        ("git-browser", "click")
            if matches!(text(body, "target"), Some("new-chat" | "new-task")) =>
        {
            text(body, "nodeId")
                .map(|path| vec![pick_folder("git-browser.folder", path)])
                .unwrap_or_default()
        }
        // Opening an HTML or SVG file with its default app (no native dialog for the Debug API).
        ("files", "external") if text(body, "what") == Some("open") => {
            let pins = vec![("/type", json!("files")), ("/request/op", json!("open"))];
            vec![Grant::new(
                "files.open",
                pin_if(pins, "/request/sessionId", body, "sessionId"),
            )]
        }
        // An export written to the destination the request names (the window uses the save dialog).
        ("project" | "pages", "export") => text(body, "destination")
            .map(|destination| {
                let pins = vec![
                    ("/type", json!("pages")),
                    ("/request/op", json!("exportPage")),
                    ("/request/destination", json!(destination)),
                ];
                vec![Grant::new(
                    "project.export",
                    pin_if(pins, "/request/path", body, "path"),
                )]
            })
            .unwrap_or_default(),
        // Full access: the native confirmation of the switch is answered as the Debug API's. A new chat holds the
        // mode until its first send, so `send` carries the same grant (the new session's id is not known yet).
        ("composer", "access") if text(body, "mode") == Some("auto-approve") => vec![access(body)],
        ("composer", "send") if matches!(text(body, "sessionId"), None | Some("new-chat")) => {
            vec![access(body)]
        }
        ("marketplace", action) => market(action, body, facts),
        ("connectors", "addServer") => text(body, "url")
            .map(|url| {
                let pins = vec![
                    ("/type", json!("connectors")),
                    ("/request/op", json!("addServer")),
                    ("/request/url", json!(url)),
                ];
                vec![Grant::new(
                    "connectors.addServer",
                    pin_if(pins, "/request/name", body, "name"),
                )]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The confirmation of a switch to Full access (`auto-approve`), pinned to the body's session when it names one.
fn access(body: &Value) -> Grant {
    let pins = vec![
        ("/type", json!("requestConfirmation")),
        ("/method", json!("session/agent/update")),
        ("/params/agentName", json!("auto-approve")),
    ];
    let pins = match text(body, "sessionId").filter(|id| *id != "new-chat") {
        Some(id) => {
            let mut pins = pins;
            pins.push(("/params/sessionId", json!(id)));
            pins
        }
        None => pins,
    };
    Grant::new("composer.access", pins)
}

/// The Marketplace tab's changes that ask the native dialog of the window.
fn market(action: &str, body: &Value, facts: Option<&dyn GrantFacts>) -> Vec<Grant> {
    let marketplace = |name: &'static str, op: &str, key: &'static str, value: &str| {
        let mut pins = vec![("/type", json!("marketplace")), ("/request/op", json!(op))];
        pins.push((
            if key == "id" {
                "/request/id"
            } else {
                "/request/path"
            },
            json!(value),
        ));
        Grant::new(name, pins)
    };
    match action {
        // Without an offer id: an install from the catalog without the offer step (an offered install needs none).
        "install" => text(body, "id")
            .map(|id| {
                let mut grant = marketplace("marketplace.install", "install", "id", id);
                grant.pins.push(("/request/offerId", Value::Null));
                vec![grant]
            })
            .unwrap_or_default(),
        "remove" => text(body, "id")
            .map(|id| vec![marketplace("marketplace.remove", "remove", "id", id)])
            .unwrap_or_default(),
        "turnOnPlugin" | "removePlugin" => text(body, "path")
            .map(|path| {
                let name = if action == "turnOnPlugin" {
                    "marketplace.turnOnPlugin"
                } else {
                    "marketplace.removePlugin"
                };
                vec![marketplace(name, action, "path", path)]
            })
            .unwrap_or_default(),
        // A catalog server: added under its catalog id, at the catalog's own address (looked up here, not taken
        // from the page). Without the catalog no grant: the window's dialog decides.
        "addServer" => text(body, "id")
            .and_then(|id| Some((id, facts?.catalog_server_url(id)?)))
            .map(|(id, url)| {
                vec![Grant::new(
                    "marketplace.addServer",
                    vec![
                        ("/type", json!("connectors")),
                        ("/request/op", json!("addServer")),
                        ("/request/url", json!(url)),
                        ("/request/name", json!(id)),
                    ],
                )]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}
