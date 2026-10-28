//! Recognizes MSYS drive spelling independently of the host OS; the Windows folder hook uses it.
//! Only a single ASCII drive letter is translated; shell mount points and relative paths retain their meaning.

pub(crate) fn native_drive(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.first() != Some(&b'/')
        || !bytes.get(1)?.is_ascii_alphabetic()
        || (bytes.len() > 2 && bytes[2] != b'/')
    {
        return None;
    }
    Some(format!(
        "{}:\\{}",
        char::from(bytes[1].to_ascii_uppercase()),
        path.get(3..).unwrap_or_default().replace('/', "\\")
    ))
}

pub(crate) fn rewrite(tool: &str, input: &serde_json::Value) -> Option<serde_json::Value> {
    let leaf = tool.rsplit('.').next().unwrap_or(tool);
    if (!tool.starts_with("file_system.") || crate::folders::SHELL_TOOLS.contains(&leaf))
        && !matches!(
            tool,
            "read_file" | "write_file" | "edit" | "search_replace" | "grep" | "glob"
        )
    {
        return None;
    }
    let mut rewritten = input.clone();
    let mut changed = false;
    for key in crate::folders::PATH_FIELDS {
        let Some(value) = rewritten.get_mut(*key) else {
            continue;
        };
        let values = match value {
            serde_json::Value::Array(items) => items.as_mut_slice(),
            value => std::slice::from_mut(value),
        };
        for value in values {
            if let Some(native) = value.as_str().and_then(native_drive) {
                *value = serde_json::Value::String(native);
                changed = true;
            }
        }
    }
    changed.then_some(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_field_and_array_is_rewritten_without_changing_other_input() {
        for tool in [
            "read_file",
            "write_file",
            "edit",
            "file_system.read_file",
            "file_system.search_replace",
            "file_system.find",
        ] {
            for key in crate::folders::PATH_FIELDS {
                for value in [
                    serde_json::json!("/c/Users/x"),
                    serde_json::json!(["/c/Users/x", "/tmp/a", 3]),
                ] {
                    let input =
                        serde_json::json!({(*key):value, "content":"/d/do not rewrite text"});
                    let mapped = rewrite(tool, &input).unwrap();
                    assert_eq!(mapped["content"], input["content"]);
                    let expected = if value.is_array() {
                        serde_json::json!(["C:\\Users\\x", "/tmp/a", 3])
                    } else {
                        serde_json::json!("C:\\Users\\x")
                    };
                    assert_eq!(mapped[*key], expected, "{tool} {key}");
                }
            }
        }
        for tool in [
            "bash",
            "file_system.bash",
            "mcp_vw.document_read",
            "mcp_other.read_file",
        ] {
            assert!(rewrite(tool, &serde_json::json!({"path":"/c/Users/x"})).is_none());
        }
    }

    #[test]
    fn drive_roots_case_and_unicode_tail() {
        for (input, expected) in [
            ("/c", "C:\\"),
            ("/C/", "C:\\"),
            (
                "/z/Users/Ilze Kalniņa/file.pdf",
                "Z:\\Users\\Ilze Kalniņa\\file.pdf",
            ),
            ("/d/a/../b", "D:\\a\\..\\b"),
        ] {
            assert_eq!(native_drive(input).as_deref(), Some(expected), "{input}");
        }
    }

    #[test]
    fn non_drive_paths_are_not_translated() {
        for input in [
            "/tmp/a",
            "/usr/bin",
            "//server/share",
            "/cygdrive/c/a",
            "relative/a",
            "c/a",
            "C:\\a",
            "C:/a",
            "/",
            "",
            "/cc/a",
            "/1/a",
            "/é/a",
            "/cousin",
            "/c\\a",
        ] {
            assert_eq!(native_drive(input), None, "{input}");
        }
    }
}
