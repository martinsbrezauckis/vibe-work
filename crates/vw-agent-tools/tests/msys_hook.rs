//! Windows MSYS hook regressions: the shipped CLI rewrites only after guarding the native destination.
//! A disposable tree and a missing copy-check endpoint prove protected paths fail closed; Linux stays unchanged.
#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use std::io::Write as _;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn hook(home: &Path, cwd: &Path, tool: &str, input: &Value) -> Option<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
            .args(["guard", "folder", "--endpoint-file"])
            .arg(home.join("absent-endpoint.json"))
            .env("VIBE_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let payload = json!({"session_id":"test", "cwd":cwd, "tool_name":tool,
        "tool_call_id":"path-test", "tool_input":input});
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        (!output.stdout.is_empty()).then(|| serde_json::from_slice(&output.stdout).unwrap())
    }

    #[cfg(windows)]
    fn msys(path: &Path) -> String {
        let native = path.to_string_lossy().replace('\\', "/");
        let native = native.strip_prefix("//?/").unwrap_or(&native);
        assert_eq!(&native[1..2], ":");
        format!("/{}{}", native[..1].to_lowercase(), &native[2..])
    }

    #[cfg(windows)]
    #[test]
    fn maps_file_fields_but_refuses_native_vibe_home_and_other_copy() {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("msys-hook-{}", std::process::id()));
        let home = root.join("vibe-home");
        let cwd = root.join("project");
        let other = home.join("worktrees/other/session");
        for dir in [&home, &cwd, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for tool in [
            "read_file",
            "write_file",
            "edit",
            "file_system.search_replace",
        ] {
            for key in vw_agent_tools::folders::PATH_FIELDS {
                let native = cwd.join("file.txt");
                let result = hook(&home, &cwd, tool, &json!({(*key):msys(&native)})).unwrap();
                assert_eq!(
                    Path::new(
                        result["hook_specific_output"]["tool_input"][*key]
                            .as_str()
                            .unwrap()
                    ),
                    native
                );
                let result = hook(
                    &home,
                    &cwd,
                    tool,
                    &json!({(*key):msys(&other.join("file.txt"))}),
                )
                .unwrap();
                assert_eq!(result["decision"], "deny", "{result}");
            }
        }
        let result = hook(
            &home,
            &cwd,
            "write_file",
            &json!({"path":msys(&home.join("config.toml"))}),
        )
        .unwrap();
        assert_eq!(result["decision"], "deny");
        for path in [
            "/tmp/a",
            "/usr/a",
            "//server/share/a",
            "/cygdrive/c/a",
            "relative.txt",
        ] {
            let result = hook(&home, &cwd, "read_file", &json!({"path":path}));
            assert!(
                result
                    .as_ref()
                    .is_none_or(|v| v.get("hook_specific_output").is_none()),
                "{path}: {result:?}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_hook_leaves_msys_spelling_unchanged() {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("msys-hook-{}", std::process::id()));
        let home = root.join("home");
        let cwd = root.join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        assert!(hook(&home, &cwd, "write_file", &json!({"path":"/c/Users/x"})).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
