//! Recognizes literal process-stop commands at command starts for the process guard.
//! Depends on `command_segments` and `killparse_cmds`; the hook and core call `scan`.
//! This is a guardrail for common commands, not a shell parser or sandbox.

use serde::{Deserialize, Serialize};

use crate::killparse_cmds as cmds;

/// How a name target matches a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameMatch {
    /// Exact executable name, ignoring a trailing `.exe`.
    Exact,
    /// Executable name contains the word.
    Substring,
}

/// A process target carried over the guard's existing wire format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum KillTarget {
    /// A process ID.
    Pid {
        /// The process ID.
        pid: u32,
    },
    /// A process group ID, retained for existing wire readers.
    Group {
        /// The process group ID.
        pgid: u32,
    },
    /// All processes of the user, retained for existing wire readers.
    AllProcesses,
    /// A process name.
    Name {
        /// The name as written.
        name: String,
        /// How the process name is matched.
        how: NameMatch,
    },
    /// A listening TCP port, retained for existing wire readers.
    Port {
        /// The TCP port.
        port: u16,
    },
}

/// One scan of the command text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillScan {
    /// A recognized process-stop command starts a segment.
    pub mentions: bool,
    /// Literal targets; complete only when `unparseable` is empty.
    pub targets: Vec<KillTarget>,
    /// Reasons for asking the user to decide.
    pub unparseable: Vec<String>,
}

impl KillScan {
    /// Whether all targets of a recognized command were read literally.
    #[must_use]
    pub fn understood(&self) -> bool {
        self.mentions && self.unparseable.is_empty()
    }

    pub(crate) fn target(&mut self, target: KillTarget) {
        if !self.targets.contains(&target) {
            self.targets.push(target);
        }
    }

    pub(crate) fn reason(&mut self, why: impl Into<String>) {
        let why = why.into();
        if !self.unparseable.contains(&why) {
            self.unparseable.push(why);
        }
    }
}

/// The literal basename of a program token, including Windows paths.
pub(crate) fn command_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = base.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

/// Reads command starts after assignments and transparent wrappers in the unquoted literal, Bash and Windows readings
/// (`gitparse::readings`): a quoted program name or a quoted argument with a space is read like the shell reads
/// it, and every target or reason any reading finds counts.
#[must_use]
pub fn scan(command: &str) -> KillScan {
    let mut out = KillScan::default();
    // Process targets retain the conservative literal reading: a quoted PID still asks the user.
    let literal = crate::command_segments::segments(command)
        .into_iter()
        .map(|segment| segment.into_iter().map(str::to_owned).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    for reading in std::iter::once(literal).chain(crate::gitparse::readings(command)) {
        for segment in &reading {
            let segment: Vec<&str> = segment.iter().map(String::as_str).collect();
            scan_segment(&segment, &mut out);
        }
    }
    out
}

fn scan_segment(segment: &[&str], out: &mut KillScan) {
    let words = crate::command_segments::command_words(segment);
    let Some(program) = words.first() else {
        return;
    };
    let args = &words[1..];
    match command_name(program).as_str() {
        "kill" => cmds::kill(args, out),
        "pkill" => cmds::pkill(args, out),
        "killall" => cmds::killall(args, out),
        "taskkill" => cmds::taskkill(args, out),
        "stop-process" | "spps" => cmds::stop_process(args, out),
        _ => return,
    }
    out.mentions = true;
}
