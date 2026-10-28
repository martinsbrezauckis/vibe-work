//! The `gh` wrapper's process cleanup on Linux, with a stand-in `gh` (a shell script that starts a background child
//! and waits). Reads `/proc` to tell a live process from a gone or zombie one.
use std::path::Path;
use std::time::Duration;

/// Whether `pid` is a running process (not gone, not a zombie waiting to be reaped).
fn alive(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, rest)| !rest.starts_with('Z') && !rest.starts_with('X'))
    })
}

/// Waits up to five seconds for `file` to hold a pid.
async fn pid_in(file: &Path) -> String {
    for _ in 0..500 {
        if let Ok(pid) = std::fs::read_to_string(file)
            && !pid.trim().is_empty()
        {
            return pid.trim().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} never held a pid", file.display())
}

/// A call whose future is dropped (a closed window's command, a cancelled task, runtime shutdown) stops gh and every
/// process gh started in its group, as the timeout does.
#[tokio::test]
async fn a_dropped_gh_call_stops_every_process_gh_started() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/gh-dropped-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (leader, helper) = (dir.join("leader.pid"), dir.join("helper.pid"));
    let _ = std::fs::remove_file(&leader);
    let _ = std::fs::remove_file(&helper);
    let program = dir.join("gh");
    // Not std::fs::write + set_permissions: another test thread forking while this process holds the file open for
    // writing makes the exec fail with "Text file busy" (ETXTBSY), and the stand-in never runs (about 1 in 5 full lib
    // runs). `install` writes it from a separate process, so no file of this process is open for writing.
    let source = dir.join("gh.source");
    std::fs::write(
        &source,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nsleep 300 &\necho $! > '{}'\nexec sleep 300\n",
            leader.display(),
            helper.display()
        ),
    )
    .unwrap();
    let installed = std::process::Command::new("install")
        .args(["-m", "0755"])
        .arg(&source)
        .arg(&program)
        .status()
        .unwrap();
    assert!(installed.success(), "install the stand-in gh");
    let call = tokio::spawn({
        let program = program.clone();
        async move { super::gh(&program, &["auth", "status"]).await }
    });
    let (leader, helper) = (pid_in(&leader).await, pid_in(&helper).await);
    assert!(alive(&leader) && alive(&helper), "the stand-in gh runs");
    call.abort();
    let _ = call.await;
    let mut gone = false;
    for _ in 0..200 {
        if !alive(&leader) && !alive(&helper) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let left = (alive(&leader), alive(&helper));
    for pid in [&leader, &helper] {
        let _ = std::process::Command::new("kill")
            .args(["-9", pid])
            .status();
    }
    assert!(
        gone,
        "after the dropped call: gh alive {}, its background child alive {}",
        left.0, left.1
    );
}
