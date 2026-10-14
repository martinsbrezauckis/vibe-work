//! Literal argument readers for process-stop commands recognized by `killparse`.
//! Depends on `killparse` target types; called only after a recognized command start.

use crate::killparse::{KillScan, KillTarget, NameMatch};

fn pid(text: &str, out: &mut KillScan) {
    match text.parse::<u32>() {
        Ok(pid) if pid > 0 => out.target(KillTarget::Pid { pid }),
        _ => out.reason("a process ID is not a literal positive number"),
    }
}

fn name(text: &str, how: NameMatch, out: &mut KillScan) {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        out.target(KillTarget::Name {
            name: text.to_owned(),
            how,
        });
    } else {
        out.reason("a process name is not literal");
    }
}

fn signal(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
    matches!(
        bare,
        "TERM"
            | "KILL"
            | "HUP"
            | "INT"
            | "QUIT"
            | "STOP"
            | "CONT"
            | "USR1"
            | "USR2"
            | "ABRT"
            | "ALRM"
    ) || text.parse::<u8>().is_ok_and(|n| n <= 64)
}

pub(crate) fn kill(args: &[&str], out: &mut KillScan) {
    if args == ["-l"] {
        return;
    }
    if args
        .iter()
        .any(|a| matches!(a.to_ascii_lowercase().as_str(), "-id" | "-name"))
    {
        stop_process(args, out);
        return;
    }
    let mut i = 0;
    let mut target = false;
    let mut options = true;
    while i < args.len() {
        let a = args[i];
        if options && a == "--" {
            options = false;
            i += 1;
            continue;
        }
        if options && !target && matches!(a, "-s" | "-n" | "--signal") {
            i += 1;
            if !args.get(i).is_some_and(|s| signal(s)) {
                out.reason("a signal is not literal");
                return;
            }
        } else if options && !target && a.starts_with("--signal=") {
            if !signal(&a[9..]) {
                out.reason("a signal is not literal");
            }
        } else if options && !target && a.starts_with('-') && signal(&a[1..]) {
            // A signal option before a target.
        } else {
            target = true;
            pid(a, out);
        }
        i += 1;
    }
    if !target {
        out.reason("kill has no literal process target");
    }
}

pub(crate) fn pkill(args: &[&str], out: &mut KillScan) {
    let how = if args
        .iter()
        .take_while(|a| **a != "--")
        .any(|a| matches!(*a, "-x" | "--exact"))
    {
        NameMatch::Exact
    } else {
        NameMatch::Substring
    };
    let mut found = false;
    let mut i = 0;
    let mut options = true;
    while i < args.len() {
        match args[i] {
            "--" if options => options = false,
            "-x" | "--exact" if options => {}
            "--signal" if options => {
                i += 1;
                if !args.get(i).is_some_and(|s| signal(s)) {
                    out.reason("a signal is not literal");
                    return;
                }
            }
            a if options && a.starts_with('-') && signal(&a[1..]) => {}
            a if options && a.starts_with('-') => out.reason("pkill has an unsupported option"),
            a => {
                if found {
                    out.reason("pkill has multiple patterns");
                }
                found = true;
                // pkill uses a regex; a dot is not a literal name character there.
                if a.contains('.') {
                    out.reason("pkill has a nonliteral process pattern");
                } else {
                    name(a, how, out);
                }
            }
        }
        i += 1;
    }
    if !found {
        out.reason("pkill has no literal process name");
    }
}

pub(crate) fn killall(args: &[&str], out: &mut KillScan) {
    let mut found = false;
    let mut options = true;
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        match a {
            "--" if options => options = false,
            "-s" | "--signal" if options => {
                i += 1;
                if !args.get(i).is_some_and(|s| signal(s)) {
                    out.reason("a signal is not literal");
                    return;
                }
            }
            a if options && a.starts_with('-') && signal(&a[1..]) => {}
            a if options && a.starts_with('-') => out.reason("killall has an unsupported option"),
            a => {
                found = true;
                name(a, NameMatch::Exact, out);
            }
        }
        i += 1;
    }
    if !found {
        out.reason("killall has no literal process name");
    }
}

pub(crate) fn taskkill(args: &[&str], out: &mut KillScan) {
    let mut found = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].to_ascii_lowercase().as_str() {
            "/f" => {}
            "/pid" | "/im" => {
                let is_pid = args[i].eq_ignore_ascii_case("/pid");
                i += 1;
                if let Some(value) = args.get(i) {
                    found = true;
                    if is_pid {
                        pid(value, out);
                    } else {
                        name(value, NameMatch::Exact, out);
                    }
                } else {
                    out.reason("taskkill has a target option without a value");
                }
            }
            _ => out.reason("taskkill has an unsupported option"),
        }
        i += 1;
    }
    if !found {
        out.reason("taskkill has no literal target");
    }
}

pub(crate) fn stop_process(args: &[&str], out: &mut KillScan) {
    let mut found = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_ascii_lowercase();
        match a.as_str() {
            "-force" => {}
            "-id" | "-name" | "-processname" => {
                let is_pid = a == "-id";
                i += 1;
                if let Some(value) = args.get(i) {
                    found = true;
                    for part in value.split(',') {
                        if is_pid {
                            pid(part, out);
                        } else {
                            name(part, NameMatch::Exact, out);
                        }
                    }
                } else {
                    out.reason("Stop-Process has a target option without a value");
                }
            }
            a if a.starts_with('-') => out.reason("Stop-Process has an unsupported option"),
            _ => {
                found = true;
                pid(args[i], out);
            }
        }
        i += 1;
    }
    if !found {
        out.reason("Stop-Process has no literal target");
    }
}
