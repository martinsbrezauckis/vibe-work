//! Launch-command regressions, using the real argv builder and Windows std quoting on every host.
//! The recorder shape follows tests/public/rust/tests/common/core.rs; no Vibe process is needed.

use super::*;
use std::ffi::OsStr;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    config: SupervisorConfig,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
        let root = base.join(format!("launch command {name} 🦀-{}", std::process::id()));
        let bin = root.join(if cfg!(windows) {
            "venv/Scripts"
        } else {
            "venv/bin"
        });
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.join("venv/Lib/site-packages/vibe")).unwrap();
        std::fs::write(root.join("venv/pyvenv.cfg"), "").unwrap();
        let program = bin.join(crate::BINARY_NAME);
        std::fs::write(&program, "#!/usr/bin/python\n").unwrap();
        let python = if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        };
        std::fs::write(bin.join(python), "").unwrap();
        let mut config = SupervisorConfig::new(program, root.join("vibe-home"), root.join("work"));
        config.extra_args = vec![
            "--additional-builtin-plugin-root".into(),
            root.join("resources/plugin").into_os_string(),
        ];
        Self { root, config }
    }

    fn argv(&self) -> Vec<OsString> {
        server_argv(&self.config, &self.config.program).unwrap().0
    }

    fn record(&mut self) {
        self.config
            .launcher
            .push(self.root.join("vibe_wire_tap.exe").into_os_string());
        for (flag, file) in [
            ("methods", "methods.log"),
            ("channels", "channels.log"),
            ("params", "params.jsonl"),
            ("requests", "requests.jsonl"),
            ("responses", "responses.jsonl"),
            ("launch", "launch.json"),
            ("agent-config", "agent-config.jsonl"),
            ("plugin-roots", "vibe-home/plugin-roots.jsonl"),
        ] {
            self.config.launcher.extend([
                format!("--{flag}").into(),
                self.root.join(file).into_os_string(),
            ]);
        }
        self.config.launcher.extend([
            "--".into(),
            adapter_python(&self.config.program)
                .unwrap()
                .into_os_string(),
            "-I".into(),
            "-u".into(),
            self.root.join("process_start_barrier.py").into_os_string(),
        ]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn guard_capability_follows_the_actual_command_branch() {
    let fixture = Fixture::new("guard-capability");
    let (_, guarded) = server_argv(&fixture.config, &fixture.config.program).unwrap();
    assert!(guarded);
    let python = adapter_python(&fixture.config.program).unwrap();
    std::fs::remove_file(python).unwrap();
    let error = server_argv(&fixture.config, &fixture.config.program).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot run with Vibe Work's safety guards")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_unproven_vibe_script_is_refused_before_its_process_starts() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("unproven-script");
    let python = fixture.config.program.parent().unwrap().join("python");
    std::fs::remove_file(python).unwrap();
    let marker = fixture.root.join("started");
    std::fs::write(
        &fixture.config.program,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(
        &fixture.config.program,
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let config = SupervisorConfig::new(
        fixture.config.program.clone(),
        fixture.root.join("home"),
        fixture.root.clone(),
    );

    let Err(error) = crate::Supervisor::start(config, std::sync::Arc::new(crate::NoSecret)).await
    else {
        panic!("unproven Vibe script started")
    };
    assert!(!marker.exists(), "unproven script ran before refusal");
    assert_eq!(
        crate::start_failure_message(&error),
        "This Vibe installation cannot run with Vibe Work's safety guards: choose Install Vibe for Vibe Work, or a Vibe installed in its own Python environment."
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_named_vibe_symlink_cannot_hide_an_unproven_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture = Fixture::new("named-symlink");
    std::fs::remove_file(&fixture.config.program).unwrap();
    std::fs::remove_file(fixture.config.program.parent().unwrap().join("python")).unwrap();
    let target = fixture.root.join("wrapper");
    let marker = fixture.root.join("wrapper-started");
    std::fs::write(
        &target,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&target, &fixture.config.program).unwrap();
    let config = SupervisorConfig::new(
        fixture.config.program.clone(),
        fixture.root.join("home"),
        fixture.root.clone(),
    );

    let Err(error) = crate::Supervisor::start(config, std::sync::Arc::new(crate::NoSecret)).await
    else {
        panic!("symlinked Vibe script started")
    };
    assert!(!marker.exists(), "symlink target ran without the adapter");
    assert!(matches!(error, SupervisorError::UnprovenInterpreter));
}

#[cfg(unix)]
#[test]
fn a_venv_shebang_without_a_sibling_uses_the_adapter() {
    let fixture = Fixture::new("venv-shebang");
    std::fs::remove_file(fixture.config.program.parent().unwrap().join("python")).unwrap();
    let venv = std::env::temp_dir().join(format!("vw-venv-shebang-{}", std::process::id()));
    let bin = venv.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(venv.join("pyvenv.cfg"), "").unwrap();
    let python = bin.join("python3");
    std::fs::write(&python, "").unwrap();
    std::fs::write(&fixture.config.program, format!("#!{}\n", python.display())).unwrap();
    let (argv, guarded) = server_argv(&fixture.config, &fixture.config.program).unwrap();
    assert!(guarded);
    assert_eq!(argv[0], python.canonicalize().unwrap().as_os_str());
    assert_eq!(&argv[1..4], &["-I", "-u", "-c"]);
    assert_eq!(argv[4], crate::ADAPTER_BOOTSTRAP);
    assert_eq!(argv[7], fixture.config.program.as_os_str());
    std::fs::remove_dir_all(venv).unwrap();
}

#[cfg(unix)]
#[test]
fn an_env_shebang_without_a_sibling_is_refused() {
    let fixture = Fixture::new("env-shebang");
    std::fs::remove_file(fixture.config.program.parent().unwrap().join("python")).unwrap();
    std::fs::write(&fixture.config.program, "#!/usr/bin/env python3\n").unwrap();
    let error = server_argv(&fixture.config, &fixture.config.program).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot run with Vibe Work's safety guards")
    );
}

#[cfg(unix)]
#[test]
fn malformed_or_unassociated_shebangs_do_not_prove_an_interpreter() {
    let fixture = Fixture::new("unassociated-shebangs");
    std::fs::remove_file(fixture.config.program.parent().unwrap().join("python")).unwrap();
    let bin = fixture.root.join("other/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let python = bin.join("python3");
    std::fs::write(&python, "").unwrap();
    for shebang in [
        "#!relative/python3\n".to_owned(),
        format!("#!{} -E\n", python.display()),
        format!("#!{}\n", python.display()),
        format!("#!{}", python.display()),
        "#!/definitely/missing/python3\n".to_owned(),
    ] {
        std::fs::write(&fixture.config.program, shebang).unwrap();
        assert!(
            server_argv(&fixture.config, &fixture.config.program).is_err(),
            "unassociated shebang accepted"
        );
    }
}

#[test]
fn a_test_server_with_another_name_keeps_direct_launch() {
    let fixture = Fixture::new("fake-server-name");
    let program = fixture.root.join("fake-server");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    let (argv, guarded) = server_argv(&fixture.config, &program).unwrap();
    assert!(!guarded);
    assert_eq!(argv[0], program.as_os_str());
}

#[test]
fn an_unrecognized_windows_server_launcher_is_refused() {
    let fixture = Fixture::new("unrecognized-exe");
    let program = fixture.root.join("vibe-app-server.exe");
    std::fs::write(&program, b"MZ not a recognized Vibe launcher").unwrap();
    let config = SupervisorConfig::new(
        program.clone(),
        fixture.config.vibe_home.clone(),
        fixture.root.clone(),
    );
    let error = server_argv(&config, &program).unwrap_err();
    assert!(matches!(error, SupervisorError::UnprovenInterpreter));
}

#[test]
fn a_pending_supervisor_does_not_advertise_an_installed_guard() {
    let fixture = Fixture::new("pending-guard");
    let supervisor = crate::Supervisor::start_pending(
        SupervisorConfig::new(
            fixture.config.program.clone(),
            fixture.config.vibe_home.clone(),
            fixture.root.clone(),
        ),
        std::sync::Arc::new(crate::NoSecret),
        "not started".into(),
    );
    assert!(!supervisor.mcp_refresh_guard_installed());
}

// std's Windows make_command_line always quotes argv[0]. append_arg quotes empty/space/tab
// arguments, escapes quotes with 2n+1 backslashes, and doubles trailing slashes inside quotes.
// Count UTF-16, including the terminating NUL, rather than UTF-8 bytes or Unicode scalar values.
fn windows_command_line(argv: &[OsString]) -> Vec<u16> {
    fn wide(value: &OsStr) -> Vec<u16> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            value.encode_wide().collect()
        }
        #[cfg(not(windows))]
        value.to_str().unwrap().encode_utf16().collect()
    }
    let quote = u16::from(b'"');
    let slash = u16::from(b'\\');
    let mut line = vec![quote];
    line.extend(wide(&argv[0]));
    line.push(quote);
    for arg in &argv[1..] {
        line.push(u16::from(b' '));
        let arg = wide(arg);
        let quoted = arg.is_empty() || arg.iter().any(|&c| c == 32 || c == 9);
        if quoted {
            line.push(quote);
        }
        let mut slashes = 0;
        for c in arg {
            if c == slash {
                slashes += 1;
            } else {
                if c == quote {
                    line.extend(std::iter::repeat_n(slash, slashes + 1));
                }
                slashes = 0;
            }
            line.push(c);
        }
        if quoted {
            line.extend(std::iter::repeat_n(slash, slashes));
            line.push(quote);
        }
    }
    line.push(0);
    line
}

#[test]
fn windows_quoting_counts_spaces_quotes_trailing_slashes_and_surrogate_pairs() {
    let argv = ["python.exe", "", "a b\\", "a\\\"b", "🦀"];
    let actual = windows_command_line(&argv.map(OsString::from));
    let expected = "\"python.exe\" \"\" \"a b\\\\\" a\\\\\\\"b 🦀\0";
    assert_eq!(actual, expected.encode_utf16().collect::<Vec<_>>());
}

#[cfg(windows)]
#[test]
fn windows_measurement_matches_the_command_line_received_by_a_real_child() {
    let python = std::env::var_os("VIBE_PYTHON").unwrap_or_else(|| "python.exe".into());
    let probe = "import ctypes,sys; f=ctypes.windll.kernel32.GetCommandLineW; f.restype=ctypes.c_wchar_p; sys.stdout.buffer.write(f().encode('utf-16-le'))";
    let argv: Vec<OsString> = std::iter::once(python)
        .chain(
            [
                "-I", "-c", probe, "", "a b\\", "a\\\"b", "🦀", "a\tb", "a\nb",
            ]
            .map(OsString::from),
        )
        .collect();
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut received: Vec<u16> = output
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    received.push(0);
    assert_eq!(windows_command_line(&argv), received);
}

#[test]
fn launch_commands_leave_room_for_windows_paths_and_recording_wrappers() {
    let mut fixture = Fixture::new("length");
    let direct = windows_command_line(&fixture.argv()).len();
    fixture.record();
    let recorded = windows_command_line(&fixture.argv()).len();
    println!("Windows UTF-16 units including NUL: product={direct}, recorder+wrapper={recorded}");
    // Keep >24,767 units spare below CreateProcessW's 32,767-unit ceiling. This budget
    // includes eight recorder paths, a process-tracking wrapper and a plugin root.
    assert!(
        direct < 8_000 && recorded < 8_000,
        "product={direct}, recorder+wrapper={recorded}"
    );
}

#[test]
fn adapter_source_never_travels_on_argv() {
    let fixture = Fixture::new("source");
    let argv = fixture.argv();
    assert!(
        !argv
            .iter()
            .any(|arg| arg == CHILD_STOP_ADAPTER || arg.to_string_lossy().contains("def install")),
        "the Python adapter must be delivered outside argv"
    );
}

#[test]
fn a_guard_list_that_does_not_read_as_plain_versions_offers_nothing() {
    let read = |line: &str| super::parse_guarded(&format!("x = 1\n{line}\n"));
    assert_eq!(
        read(r#"GUARDED_VIBE_VERSIONS = frozenset({"3.1.2", "3.1.3"})"#),
        ["3.1.2", "3.1.3"]
    );
    for line in [
        r#"GUARDED_VIBE_VERSIONS = frozenset({"3.1.2", "3.1.x"})"#,
        r#"GUARDED_VIBE_VERSIONS = frozenset({"3.1.2", OTHER})"#,
        r#"GUARDED_VIBE_VERSIONS = frozenset({"3.1.2"}) | EXTRA"#,
        r#"GUARDED_VIBE_VERSIONS = {"3.1.2"}"#,
        "GUARDED_VIBE_VERSIONS = frozenset({})",
    ] {
        assert!(read(line).is_empty(), "{line}");
    }
}
