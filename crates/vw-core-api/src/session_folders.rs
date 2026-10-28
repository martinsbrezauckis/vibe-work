//! Checks a session's copy folder before adopting its opened state. Depends on JSON and the folder resolver;
//! called by sessions. The cwd must remain inside the copy recorded by the session.

use std::path::Path;

use serde_json::Value;
use vw_agent_tools::folders::{Layout, resolve};

pub(crate) fn check(home: &Path, state: &Value, previous: Option<&Value>) -> Result<(), String> {
    let Some(cwd) = state
        .pointer("/session/cwd")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    Layout::new(home, Path::new(cwd)).map_err(|e| e.to_string())?;
    let copy = previous
        .and_then(|v| v.pointer("/session/worktree/path"))
        .and_then(Value::as_str)
        .or_else(|| {
            state
                .pointer("/session/worktree/path")
                .and_then(Value::as_str)
        });
    if let Some(copy) = copy {
        if !Path::new(copy).is_absolute() || !Path::new(cwd).is_absolute() {
            return Err("session copy and cwd must be absolute".to_owned());
        }
        let root = resolve(Path::new(copy)).map_err(|e| e.to_string())?;
        let here = resolve(Path::new(cwd)).map_err(|e| e.to_string())?;
        if !here.starts_with(&root) {
            return Err(format!(
                "session folder {cwd} resolves outside its copy {copy}"
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use serde_json::json;
    #[test]
    fn a_copy_cwd_cannot_resolve_outside_the_recorded_copy() {
        let dir = std::env::temp_dir().join(format!("vw-copy-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("vibe-home");
        let copy = dir.join("copy");
        let cwd = copy.join("nested");
        let outside = dir.join("outside");
        for path in [&home, &cwd, &outside] {
            std::fs::create_dir_all(path).unwrap();
        }
        let state = json!({"session":{"cwd":cwd,"worktree":{"path":copy}}});
        let next = json!({"session":{"cwd":cwd}});
        std::fs::remove_dir(&cwd).unwrap();
        std::os::unix::fs::symlink(&outside, &cwd).unwrap();
        for (input, previous) in [(&state, None), (&next, Some(&state))] {
            assert!(
                super::check(&home, input, previous)
                    .unwrap_err()
                    .contains("outside its copy")
            );
        }
        std::fs::remove_file(&cwd).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        assert!(super::check(&home, &state, None).is_ok());
        assert!(super::check(&home, &next, Some(&state)).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
