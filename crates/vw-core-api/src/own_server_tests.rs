//! Process snapshots distinguish the supervisor's launchers from a nested server on every host OS.
//! Exercises the Windows image order without depending on a native process scan.
use super::*;
use std::{collections::BTreeMap, time::Duration};
use vw_procmon::{SelfInfo, StartToken};

fn snapshot(rows: &[(u32, u32, &str, bool)]) -> Snapshot {
    Snapshot {
        listeners: vec![],
        me: SelfInfo { pid: 1, pgid: None },
        took: Duration::ZERO,
        processes: rows
            .iter()
            .map(|&(pid, ppid, exe, named)| {
                (
                    pid,
                    ProcessInfo {
                        pid,
                        ppid: Some(ppid),
                        pgid: None,
                        start: Some(StartToken(u64::from(pid))),
                        same_user: Some(true),
                        name: String::new(),
                        exe: Some(PathBuf::from(exe)),
                        cmdline: if named {
                            vec!["python".into(), "/vibe/server.exe".into()]
                        } else {
                            vec!["launcher".into()]
                        },
                        cwd: None,
                        alive: true,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}
fn ours(snapshot: &Snapshot, pid: u32, leader: u32) -> bool {
    ours_under(snapshot, pid, leader, &[])
}
fn ours_under(snapshot: &Snapshot, pid: u32, leader: u32, launchers: &[&str]) -> bool {
    let launchers: Vec<PathBuf> = launchers.iter().map(PathBuf::from).collect();
    ancestors(snapshot, pid, leader).is_some_and(|chain| {
        owns(
            snapshot,
            chain,
            leader,
            Path::new("/vibe/server.exe"),
            &launchers,
            &[
                "/vibe/server.exe".into(),
                "/env/Scripts/python.exe".into(),
                "/base/python.exe".into(),
            ],
        )
    })
}
#[test]
fn child_server_our_server_launcher_snapshot_is_foreign() {
    let snapshot = snapshot(&[
        (10, 1, "/launcher", false),
        (20, 10, "/vibe/server.exe", true),
        (30, 20, "/vibe/server.exe", true),
    ]);
    assert!(!ours(&snapshot, 30, 10));
    assert!(!ours(&snapshot, 30, 20));
}
#[test]
fn only_the_spawned_launcher_and_its_ordered_interpreters_are_ours() {
    let snapshot = snapshot(&[
        (10, 1, "/vibe/server.exe", true),
        (20, 10, "/env/Scripts/python.exe", true),
        (30, 20, "/base/python.exe", true),
        (40, 30, "/vibe/server.exe", true),
    ]);
    assert!(ours(&snapshot, 10, 10));
    assert!(ours(&snapshot, 20, 10));
    assert!(ours(&snapshot, 30, 10));
    assert!(!ours(&snapshot, 40, 10));
    assert!(!ours(&snapshot, 30, 1));
}
#[test]
fn repeated_or_reordered_images_and_unreadable_owners_are_foreign() {
    for image in ["/vibe/server.exe", "/base/python.exe", "/shell"] {
        let snapshot = snapshot(&[(10, 1, "/vibe/server.exe", true), (20, 10, image, true)]);
        assert!(!ours(&snapshot, 20, 10));
    }
    let mut snapshot = snapshot(&[
        (10, 1, "/vibe/server.exe", true),
        (20, 10, "/env/Scripts/python.exe", true),
    ]);
    snapshot.processes.get_mut(&20).unwrap().same_user = None;
    assert!(!ours(&snapshot, 20, 10));
}
#[test]
fn only_an_executable_or_interpreter_script_position_names_the_server() {
    let program = Path::new("/test/vibe-app-server");
    assert!(runs_program(&["/test/vibe-app-server".into()], program));
    assert!(runs_program(
        &["python".into(), "/test/vibe-app-server".into()],
        program
    ));
    assert!(!runs_program(
        &["tap".into(), "--".into(), "/test/vibe-app-server".into()],
        program
    ));
    assert!(!runs_program(&["/other/vibe-app-server".into()], program));
    assert!(runs_program(
        &["vibe-app-server".into(), String::new(), String::new()],
        program
    ));
    assert!(!runs_program(
        &["vibe-app-server".into(), "another-script".into()],
        program
    ));
}
#[test]
fn titled_server_still_requires_the_exact_image_and_launcher_chain() {
    let mut snapshot = snapshot(&[
        (10, 1, "/tap", false),
        (20, 10, "/vibe/server.exe", true),
        (30, 20, "/vibe/server.exe", true),
        (40, 10, "/unrelated", true),
    ]);
    for pid in [20, 30, 40] {
        snapshot.processes.get_mut(&pid).unwrap().cmdline =
            vec!["vibe-app-server".into(), String::new()];
    }
    assert!(ours_under(&snapshot, 20, 10, &["/tap"]));
    assert!(!ours_under(&snapshot, 30, 10, &["/tap"]));
    assert!(!ours_under(&snapshot, 40, 10, &["/tap"]));
    assert!(!ours_under(&snapshot, 20, 10, &[]));
}
#[test]
fn configured_launchers_are_ours_only_from_the_spawned_process_down_to_the_server() {
    let snapshot = snapshot(&[
        (10, 1, "/fault", false),
        (15, 10, "/tap", false),
        (20, 15, "/vibe/server.exe", true),
        (30, 20, "/vibe/server.exe", true),
        (40, 10, "/shell", false),
        (50, 40, "/vibe/server.exe", true),
        (60, 20, "/tap", false),
        (70, 60, "/vibe/server.exe", true),
    ]);
    let launchers = ["/fault", "/tap"];
    assert!(ours_under(&snapshot, 20, 10, &launchers));
    // A server nested below ours (directly or under a launcher image it started), or started by another child of
    // the launcher, stays foreign.
    assert!(!ours_under(&snapshot, 30, 10, &launchers));
    assert!(!ours_under(&snapshot, 70, 10, &launchers));
    assert!(!ours_under(&snapshot, 50, 10, &launchers));
    // Without that configuration, or when the tap is not one of its executables, the chain is not ours.
    assert!(!ours_under(&snapshot, 20, 10, &[]));
    assert!(!ours_under(&snapshot, 20, 10, &["/fault"]));
    assert!(!ours_under(&snapshot, 20, 1, &launchers));
}
#[cfg(unix)]
#[test]
fn a_unix_script_server_is_matched_by_the_interpreter_its_first_line_names() {
    let dir = std::env::temp_dir().join(format!("vw-own-server-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("vibe-app-server");
    std::fs::write(&script, "#!/bin/sh -e\nexit 0\n").unwrap();
    let binary = dir.join("native");
    std::fs::write(&binary, b"\x7fELF").unwrap();
    assert_eq!(
        launcher_images(&script),
        [std::fs::canonicalize("/bin/sh").unwrap()]
    );
    assert_eq!(launcher_images(&binary), std::slice::from_ref(&binary));
    std::fs::remove_dir_all(&dir).unwrap();
}
/// The command line of the adapter launch: `<env python> -P -c <adapter> <program> <mode args>`.
fn adapter_cmdline(program: &Path) -> Vec<String> {
    vec![
        "/env/Scripts/python.exe".into(),
        "-P".into(),
        "-c".into(),
        vw_supervisor::CHILD_STOP_ADAPTER.into(),
        program.display().to_string(),
        "--multiplex".into(),
    ]
}
/// Windows through the child-Stop adapter: the supervisor spawns the environment's `python.exe`; that
/// launcher starts the base Python with the same command line, and the base Python's pid is what Vibe reports as a
/// child session's lease holder. Both are ours; a process below them, another `-c` program, or the adapter run for
/// another server is not.
#[test]
fn the_adapter_launch_is_ours_from_the_environment_python_down_to_its_base_python() {
    let program = Path::new("/env/Scripts/vibe-app-server.exe");
    let python = PathBuf::from("/env/Scripts/python.exe");
    let base = PathBuf::from("/base/python.exe");
    let images = image_order(program, Some(python.clone()), Some(python), Some(base));
    let mut snapshot = snapshot(&[
        (10, 1, "/env/Scripts/python.exe", true),
        (20, 10, "/base/python.exe", true),
        (30, 20, "/base/python.exe", true),
    ]);
    for pid in [10, 20, 30] {
        snapshot.processes.get_mut(&pid).unwrap().cmdline = adapter_cmdline(program);
    }
    let owned = |snapshot: &Snapshot, pid| {
        ancestors(snapshot, pid, 10)
            .is_some_and(|chain| owns(snapshot, chain, 10, program, &[], &images))
    };
    assert!(owned(&snapshot, 10), "the spawned environment Python");
    assert!(
        owned(&snapshot, 20),
        "the base Python it starts (the lease holder)"
    );
    assert!(!owned(&snapshot, 30), "a Python nested below our server");
    for (index, other) in [
        (3, "print(1)"),
        (4, "/other/vibe-app-server.exe"),
        (1, "-I"),
    ] {
        let mut changed = snapshot.clone();
        changed.processes.get_mut(&20).unwrap().cmdline[index] = other.into();
        assert!(!owned(&changed, 20), "argument {index} = {other}");
    }
}
/// The image order with and without the adapter: through it the environment's Python is the spawned image (listed
/// once), without it the entry point comes first.
#[test]
fn the_adapter_launch_starts_the_image_order_at_the_environment_python() {
    let program = Path::new("/env/Scripts/vibe-app-server.exe");
    let python = PathBuf::from("/env/Scripts/python.exe");
    let base = PathBuf::from("/base/python.exe");
    assert_eq!(
        image_order(
            program,
            Some(python.clone()),
            Some(python.clone()),
            Some(base.clone())
        ),
        [python.clone(), base.clone()]
    );
    assert_eq!(
        image_order(program, None, Some(python.clone()), Some(base.clone())),
        [program.to_owned(), python, base]
    );
}
/// macOS framework Pythons (Homebrew, python.org): the environment's `python` links to `Versions/<v>/bin/python3.x`,
/// a stub that execs `Versions/<v>/Resources/Python.app/Contents/MacOS/Python` in the same process, so that app is the
/// image the OS reports for our server (seen on macOS with a framework Python). A Python without that app keeps its own image.
#[cfg(unix)]
#[test]
fn a_macos_framework_python_is_matched_by_the_app_its_stub_runs() {
    let dir = std::env::temp_dir().join(format!("vw-own-server-fw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let version = dir.join("Frameworks/Python.framework/Versions/3.14");
    let stub = version.join("bin/python3.14");
    let app = version.join("Resources/Python.app/Contents/MacOS/Python");
    for file in [&stub, &app] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"\xcf\xfa\xed\xfe").unwrap();
    }
    let bin = dir.join("venv/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&stub, bin.join("python")).unwrap();
    let server = bin.join("vibe-app-server");
    std::fs::write(&server, format!("#!{}\n", bin.join("python").display())).unwrap();
    let stub = std::fs::canonicalize(&stub).unwrap();
    let app = std::fs::canonicalize(&app).unwrap();
    let expected = if cfg!(target_os = "macos") {
        &app
    } else {
        &stub
    };
    assert_eq!(launcher_images(&server), std::slice::from_ref(expected));

    // Under a configured launcher (the tests' recording tap), the app image running the adapter is ours.
    let mut snapshot = snapshot(&[
        (10, 1, "/tap", false),
        (20, 10, &expected.to_string_lossy(), true),
    ]);
    let mut argv = adapter_cmdline(&server);
    argv[0] = expected.display().to_string();
    snapshot.processes.get_mut(&20).unwrap().cmdline = argv;
    let chain = ancestors(&snapshot, 20, 10).unwrap();
    assert!(owns(
        &snapshot,
        chain,
        10,
        &server,
        &["/tap".into()],
        &launcher_images(&server)
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}
