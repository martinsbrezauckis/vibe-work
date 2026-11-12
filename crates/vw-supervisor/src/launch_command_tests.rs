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
        server_argv(&self.config, &self.config.program).unwrap()
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
