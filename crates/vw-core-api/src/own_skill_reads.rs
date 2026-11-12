//! Approval policy for reading the app's shipped skill references. Used by the callback pump;
//! only exact Markdown files under the launcher's built-in plugin roots qualify, never commands.
use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn approve(roots: &[PathBuf], entry: &Value) -> bool {
    let detail = &entry["detail"];
    if detail["kind"] != "approval"
        || !matches!(
            detail["effect"]["toolName"].as_str(),
            Some("file_system.read_file" | "read_file")
        )
    {
        return false;
    }
    let input = &detail["effect"]["input"];
    let Some(path) = input
        .get("filePath")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("file_path"))
        .and_then(Value::as_str)
        .map(std::path::Path::new)
    else {
        return false;
    };
    if !path.is_absolute() || path.extension().is_none_or(|ext| ext != "md") {
        return false;
    }
    let Ok(path) = vw_platform::canonical_plain(path) else {
        return false;
    };
    if !path.is_file() {
        return false;
    }
    roots.iter().any(|root| {
        let Ok(root) = vw_platform::canonical_plain(root) else {
            return false;
        };
        // Do not canonicalize this suffix: a linked skills directory must not enlarge the trusted tree.
        path.strip_prefix(root.join("vw/skills"))
            .is_ok_and(|relative| relative.components().count() >= 2)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn own_skill_reads_are_exact_read_only_files_inside_the_shipped_tree() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let root = fixture.root.join("plugin");
        let skill = root.join("vw/skills/chrome");
        std::fs::create_dir_all(&skill).unwrap();
        let reference = skill.join("actions.md");
        std::fs::write(&reference, "Reference").unwrap();
        let entry = |tool: &str, path: &std::path::Path| {
            json!({"detail":{
                "kind":"approval","effect":{"kind":"tool","toolName":tool,"input":{"filePath":path}},
                "requiredPermissions":[{"scope":"outside_directory","invocationPattern":path}]
            }})
        };
        let roots = [root.clone()];
        assert!(approve(&roots, &entry("file_system.read_file", &reference)));
        for tool in [
            "file_system.write_file",
            "file_system.edit",
            "file_system.bash",
            "mcp_untrusted.read_file",
        ] {
            assert!(!approve(&roots, &entry(tool, &reference)), "{tool}");
        }
        for path in [
            root.join("vw/secret.md"),
            root.join("other/skills/chrome/actions.md"),
            skill.join("helper.py"),
            skill.join("missing.md"),
        ] {
            assert!(
                !approve(&roots, &entry("file_system.read_file", &path)),
                "{}",
                path.display()
            );
        }
        assert!(!approve(&[], &entry("file_system.read_file", &reference)));
        #[cfg(unix)]
        {
            let outside = fixture.root.join("outside.md");
            std::fs::write(&outside, "Outside").unwrap();
            let linked = skill.join("linked.md");
            std::os::unix::fs::symlink(outside, &linked).unwrap();
            assert!(!approve(&roots, &entry("file_system.read_file", &linked)));
        }
        fixture.finish().await;
    }
}
