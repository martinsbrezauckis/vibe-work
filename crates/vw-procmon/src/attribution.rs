//! Pure attribution: which session (if any) owns each process and each listening port, with the evidence.
//! No I/O; the monitor feeds it a snapshot plus what the core knows.
//!
//! Depends on: `types`, `registry`, `text`. Called by: `monitor`, `stop`, `explain`, tests (synthetic
//! snapshots). Rules:
//! 1. Not ours by OS user, or unreadable = "other app". Never a session.
//! 2. Facts: registered by us (pid + start token), re-attached from the ledger, or descending from such a
//!    process. Inference: cwd inside a session folder (deepest folder wins), a parent attributed to a
//!    session, or membership in a registered process group (any same-user process can join a group).
//! 3. Owner = the single session of the facts tier, else of the inference tier; two or more = ambiguous =
//!    "other app" with all candidates listed.
//! 4. A URL in a session's output corroborates only; alone it never makes an owner (a session whose server
//!    failed with "address in use" still prints the URL of the port somebody else holds).
//!
//! Provenance (who started it: us = `owned`, the agent through Vibe = `agent`) is reported separately; it
//! decides what the lifecycle may stop automatically. The [`Confidence`] of a session owner (fact or
//! inferred) decides who may stop it: the session itself only on facts (security: a
//! same-user process that merely runs in the session folder, such as the user's own dev server, is not the
//! agent's to stop); a descendant of `vibe-app-server` is not a fact for a session (it names no session), so
//! Vibe's background processes are stopped through Vibe's own `backgroundProcess/stop`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registry::{OwnProcess, ProcessClass, ProcessGroup};
use crate::text::local_urls;
use crate::types::{ProcessInfo, ProcessRef, Snapshot, SocketListener};

/// One Vibe background process as session state shows it (`/backgroundProcesses`), plus output text the
/// monitor has seen for it (fed by `ProcMon::note_output`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackgroundProcess {
    /// Vibe's `processId`.
    pub process_id: String,
    /// The command Vibe ran.
    pub command: String,
    /// Vibe's status (`running`, `orphaned`,...).
    pub status: String,
    /// Output text seen so far (tail).
    pub output: String,
}

/// What the core knows about one session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionContext {
    /// Session id.
    pub session_id: String,
    /// Its folder, worktree and workspace roots.
    pub folders: Vec<PathBuf>,
    /// Its Vibe background processes.
    pub background: Vec<BackgroundProcess>,
}

/// A process re-attached from the ledger after a restart (an `agent` process still running).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adopted {
    /// The session it belongs to.
    pub session_id: String,
    /// Its identity.
    pub process: ProcessRef,
    /// Its class as recorded.
    pub class: ProcessClass,
}

/// Everything attribution reads besides the snapshot.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// Session contexts (all live sessions: ambiguity needs all of them).
    pub sessions: Vec<SessionContext>,
    /// Our registered processes.
    pub own: Vec<OwnProcess>,
    /// Ledger entries re-attached after a restart.
    pub adopted: Vec<Adopted>,
    /// The running `vibe-app-server` processes: their descendants were started by Vibe (agent class).
    pub vibe_servers: Vec<ProcessRef>,
}

/// Who owns a process or port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Owner {
    /// A session of this app.
    #[serde(rename_all = "camelCase")]
    Session {
        /// Its id.
        session_id: String,
    },
    /// Not attributable to exactly one session.
    #[serde(rename_all = "camelCase")]
    OtherApp {
        /// Executable name from the OS-reported path, when readable (never from the command line).
        name: Option<String>,
        /// Sessions that had some evidence (ambiguous or URL-only), never chosen between.
        candidates: Vec<String>,
    },
}

impl Owner {
    /// The owning session, if any.
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Session { session_id } => Some(session_id),
            Self::OtherApp { .. } => None,
        }
    }
}

/// Why an owner was chosen (or not).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Evidence {
    /// The pid (with its start token) is registered by us for the session.
    #[serde(rename_all = "camelCase")]
    Registered {
        /// Session.
        session_id: String,
        /// Registration label.
        label: String,
    },
    /// An ancestor is registered by us for the session.
    #[serde(rename_all = "camelCase")]
    RegisteredAncestor {
        /// Session.
        session_id: String,
        /// The registered ancestor.
        ancestor_pid: u32,
    },
    /// Its process group is the registered group of the session, but it does not descend from the
    /// registered process (inference: any same-user process can join a group).
    #[serde(rename_all = "camelCase")]
    RegisteredGroup {
        /// Session.
        session_id: String,
        /// The group.
        pgid: u32,
    },
    /// Re-attached from the ledger after a restart (it, or an ancestor).
    #[serde(rename_all = "camelCase")]
    Reattached {
        /// Session.
        session_id: String,
        /// The ledger process.
        pid: u32,
    },
    /// It descends from the running `vibe-app-server` (Vibe started it: agent class). Names no session.
    #[serde(rename_all = "camelCase")]
    VibeServerDescendant {
        /// The server.
        server_pid: u32,
    },
    /// Its working directory lies inside the session's folder.
    #[serde(rename_all = "camelCase")]
    CwdInFolder {
        /// Session.
        session_id: String,
        /// The (deepest) folder.
        folder: String,
    },
    /// Its parent is attributed to the session.
    #[serde(rename_all = "camelCase")]
    ParentAttributed {
        /// Session.
        session_id: String,
        /// The parent.
        parent_pid: u32,
    },
    /// The session's background-process output mentions this port (corroboration only).
    #[serde(rename_all = "camelCase")]
    UrlInOutput {
        /// Session.
        session_id: String,
        /// Vibe's process id.
        process_id: String,
        /// The URL text.
        url: String,
    },
    /// Another OS user's process.
    ForeignUser,
    /// The OS would not tell us about the process.
    Unreadable,
    /// The socket's process could not be found (another user's, or gone).
    NoProcess,
}

/// The result for one process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attributed {
    /// Owner.
    pub owner: Owner,
    /// Evidence, facts first.
    pub evidence: Vec<Evidence>,
    /// Who started it, when proven (registry, ledger, or descending from `vibe-app-server`).
    pub provenance: Option<ProcessClass>,
    /// How the session owner was decided (`None` for "other app"). Only `fact` lets the session itself
    /// (its agent, `process_stop`) stop the process; `inferred` is for display and user-confirmed stops.
    pub confidence: Option<Confidence>,
}

/// How a session owner was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// From the facts tier: registered by this app, re-attached from the ledger, or a (start-consistent)
    /// descendant of such a process. Group membership alone is not a fact.
    Fact,
    /// From the inference tier only: cwd inside the session folder, a parent that is the session's, or
    /// membership in a registered process group.
    Inferred,
}

/// The result for one listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributedListener {
    /// The socket.
    pub listener: SocketListener,
    /// Its attribution.
    pub attributed: Attributed,
}

/// Attributes every process and listener of `snapshot`.
pub struct Attribution<'a> {
    snapshot: &'a Snapshot,
    inputs: &'a Inputs,
    memo: HashMap<u32, Attributed>,
}

impl<'a> Attribution<'a> {
    /// Prepares attribution over one snapshot.
    #[must_use]
    pub fn new(snapshot: &'a Snapshot, inputs: &'a Inputs) -> Self {
        Self {
            snapshot,
            inputs,
            memo: HashMap::new(),
        }
    }

    /// The attribution of `pid` (memoized).
    pub fn process(&mut self, pid: u32) -> Attributed {
        if let Some(done) = self.memo.get(&pid) {
            return done.clone();
        }
        // Placeholder breaks parent cycles (a corrupt table): a cycle member sees "unreadable".
        self.memo
            .insert(pid, other(None, vec![Evidence::Unreadable]));
        let result = self.compute(pid);
        self.memo.insert(pid, result.clone());
        result
    }

    /// Every listener with its attribution, URL corroboration included.
    pub fn listeners(&mut self) -> Vec<AttributedListener> {
        let snapshot = self.snapshot;
        snapshot
            .listeners
            .iter()
            .map(|listener| {
                let mut attributed = match listener.pid {
                    Some(pid) => self.process(pid),
                    None if listener.same_user == Some(false) => {
                        other(None, vec![Evidence::ForeignUser, Evidence::NoProcess])
                    }
                    None => other(None, vec![Evidence::NoProcess]),
                };
                self.corroborate(listener.port, &mut attributed);
                AttributedListener {
                    listener: listener.clone(),
                    attributed,
                }
            })
            .collect()
    }

    /// Adds `UrlInOutput` evidence for `port`; URL-only sessions become candidates of an "other app".
    fn corroborate(&self, port: u16, attributed: &mut Attributed) {
        let foreign = attributed.evidence.contains(&Evidence::ForeignUser);
        for session in &self.inputs.sessions {
            for bg in &session.background {
                if let Some(url) = local_urls(&bg.output).into_iter().find(|u| u.port == port) {
                    attributed.evidence.push(Evidence::UrlInOutput {
                        session_id: session.session_id.clone(),
                        process_id: bg.process_id.clone(),
                        url: url.text,
                    });
                    if let Owner::OtherApp { candidates, .. } = &mut attributed.owner
                        && !foreign
                        && !candidates.contains(&session.session_id)
                    {
                        candidates.push(session.session_id.clone());
                    }
                }
            }
        }
    }

    fn compute(&mut self, pid: u32) -> Attributed {
        let snapshot = self.snapshot;
        let Some(info) = snapshot.processes.get(&pid) else {
            return other(None, vec![Evidence::Unreadable]);
        };
        match info.same_user {
            Some(true) => {}
            Some(false) => return other(info.exe_name(), vec![Evidence::ForeignUser]),
            None => return other(info.exe_name(), vec![Evidence::Unreadable]),
        }
        let mut evidence = Vec::new();
        let mut facts = BTreeSet::new();
        let mut provenance = None;
        let mut inferred = BTreeSet::new();
        self.facts(
            info,
            &mut evidence,
            &mut facts,
            &mut inferred,
            &mut provenance,
        );
        if let Some((sessions, folder)) = self.deepest_folder(info) {
            for session_id in sessions {
                inferred.insert(session_id.clone());
                evidence.push(Evidence::CwdInFolder {
                    session_id,
                    folder: folder.display().to_string(),
                });
            }
        }
        if let Some(parent) = snapshot.parent_of(info) {
            let parent_pid = parent.pid;
            let parent_result = self.process(parent_pid);
            if let Some(session_id) = parent_result.owner.session() {
                inferred.insert(session_id.to_owned());
                evidence.push(Evidence::ParentAttributed {
                    session_id: session_id.to_owned(),
                    parent_pid,
                });
            }
            if provenance.is_none() {
                provenance = parent_result.provenance;
            }
        }
        let deciding = if facts.is_empty() { &inferred } else { &facts };
        let owner = if deciding.len() == 1 {
            deciding.iter().next().map_or_else(
                || other_owner(info.exe_name(), Vec::new()),
                |s| Owner::Session {
                    session_id: s.clone(),
                },
            )
        } else {
            other_owner(info.exe_name(), deciding.iter().cloned().collect())
        };
        let confidence = match owner {
            Owner::Session { .. } if !facts.is_empty() => Some(Confidence::Fact),
            Owner::Session { .. } => Some(Confidence::Inferred),
            Owner::OtherApp { .. } => None,
        };
        Attributed {
            owner,
            evidence,
            provenance,
            confidence,
        }
    }

    /// Registry, ledger and `vibe-app-server` ancestry of `info` (itself, its group leader, its ancestors).
    fn facts(
        &self,
        info: &ProcessInfo,
        evidence: &mut Vec<Evidence>,
        facts: &mut BTreeSet<String>,
        inferred: &mut BTreeSet<String>,
        provenance: &mut Option<ProcessClass>,
    ) {
        let me = info.reference();
        for own in &self.inputs.own {
            if Some(own.process) == me {
                if let Some(s) = &own.session_id {
                    facts.insert(s.clone());
                    evidence.push(Evidence::Registered {
                        session_id: s.clone(),
                        label: own.label.clone(),
                    });
                }
                provenance.get_or_insert(own.class);
            } else if let (ProcessGroup::Unix { pgid }, Some(mine)) = (&own.group, info.pgid) {
                // Group membership is NOT a fact: any same-user process in the same login session can
                // `setpgid` into a group. It is inference; descent is a fact.
                if *pgid == mine
                    && self.is_live(own.process)
                    && !self.snapshot.descends_from(info.pid, own.process)
                {
                    if let Some(s) = &own.session_id {
                        inferred.insert(s.clone());
                        evidence.push(Evidence::RegisteredGroup {
                            session_id: s.clone(),
                            pgid: mine,
                        });
                    }
                } else if self.snapshot.descends_from(info.pid, own.process) {
                    if let Some(s) = &own.session_id {
                        facts.insert(s.clone());
                        evidence.push(Evidence::RegisteredAncestor {
                            session_id: s.clone(),
                            ancestor_pid: own.process.pid,
                        });
                    }
                    provenance.get_or_insert(own.class);
                }
            } else if self.snapshot.descends_from(info.pid, own.process) {
                if let Some(s) = &own.session_id {
                    facts.insert(s.clone());
                    evidence.push(Evidence::RegisteredAncestor {
                        session_id: s.clone(),
                        ancestor_pid: own.process.pid,
                    });
                }
                provenance.get_or_insert(own.class);
            }
        }
        for adopted in &self.inputs.adopted {
            if Some(adopted.process) == me || self.snapshot.descends_from(info.pid, adopted.process)
            {
                facts.insert(adopted.session_id.clone());
                evidence.push(Evidence::Reattached {
                    session_id: adopted.session_id.clone(),
                    pid: adopted.process.pid,
                });
                provenance.get_or_insert(adopted.class);
            }
        }
        for server in &self.inputs.vibe_servers {
            if Some(*server) != me && self.snapshot.descends_from(info.pid, *server) {
                evidence.push(Evidence::VibeServerDescendant {
                    server_pid: server.pid,
                });
                provenance.get_or_insert(ProcessClass::Agent);
            }
        }
    }

    /// True when the snapshot shows `process` alive with the same start token.
    fn is_live(&self, process: ProcessRef) -> bool {
        self.snapshot
            .alive(process.pid)
            .is_some_and(|p| p.start == Some(process.start))
    }

    /// Sessions whose deepest folder contains the process's cwd (more than one = same folder twice).
    fn deepest_folder(&self, info: &ProcessInfo) -> Option<(Vec<String>, PathBuf)> {
        let cwd = normalize(info.cwd.as_deref()?);
        let mut best: Option<(usize, Vec<String>, PathBuf)> = None;
        for session in &self.inputs.sessions {
            for folder in &session.folders {
                let folder = normalize(folder);
                if !cwd.starts_with(&folder) {
                    continue;
                }
                let depth = folder.components().count();
                match &mut best {
                    Some((d, sessions, _)) if *d == depth => {
                        if !sessions.contains(&session.session_id) {
                            sessions.push(session.session_id.clone());
                        }
                    }
                    Some((d, _, _)) if *d > depth => {}
                    _ => best = Some((depth, vec![session.session_id.clone()], folder.clone())),
                }
            }
        }
        best.map(|(_, sessions, folder)| (sessions, folder))
    }
}

fn other_owner(name: Option<String>, candidates: Vec<String>) -> Owner {
    Owner::OtherApp { name, candidates }
}

fn other(name: Option<String>, evidence: Vec<Evidence>) -> Attributed {
    Attributed {
        owner: other_owner(name, Vec::new()),
        evidence,
        provenance: None,
        confidence: None,
    }
}

/// Path form used for "inside" checks: component-wise (`/w/app2` is not inside `/w/app`); on Windows
/// case-insensitive and without the `\\?\` verbatim prefix `canonicalize` adds.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    if cfg!(windows) {
        let text = path.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        PathBuf::from(text.to_lowercase())
    } else {
        path.to_path_buf()
    }
}
