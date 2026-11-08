//! Stages optional vault capability wording from the shipped plugin without editing default-build skills.
//! Called at tool startup; trusted plugin files are copied into a fresh private directory, with no links followed.
use std::path::{Path, PathBuf};

pub(crate) fn stage(root: &Path, data: &Path, line: &str) -> Result<PathBuf, String> {
    let suffix = vw_agent_tools::credential::new_credential()
        .map_err(|_| "plugin staging id unavailable")?;
    let target = data
        .join("agent-tools")
        .join(format!("plugin-{}", &suffix[..16]));
    std::fs::create_dir_all(data.join("agent-tools"))
        .map_err(|_| "plugin staging folder unavailable")?;
    std::fs::create_dir(&target).map_err(|_| "plugin staging folder unavailable")?;
    copy(root, &target, line, 0).map_err(|_| "the built-in plugin could not be staged")?;
    Ok(target)
}
fn copy(source: &Path, target: &Path, line: &str, depth: usize) -> std::io::Result<()> {
    if depth > 12 {
        return Err(std::io::Error::other("plugin depth"));
    }
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            std::fs::create_dir(&destination)?;
            copy(&entry.path(), &destination, line, depth + 1)?;
        } else if kind.is_file() {
            let bytes = std::fs::read(entry.path())?;
            let bytes = if entry.file_name() == "SKILL.md" {
                let text = String::from_utf8(bytes).map_err(std::io::Error::other)?;
                text.lines()
                    .map(|old| {
                        if old.contains("`browser_capture_secret` always refuses") {
                            let prefix: String = old
                                .chars()
                                .take_while(|c| {
                                    c.is_ascii_digit() || c.is_whitespace() || *c == '.'
                                })
                                .collect();
                            format!("{prefix}{line}")
                        } else {
                            old.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into_bytes()
            } else {
                bytes
            };
            vw_platform::write_private_new(&destination, &bytes).map_err(std::io::Error::other)?;
        } else {
            return Err(std::io::Error::other("plugin links are not copied"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn staged_words_leave_the_shipped_plugin_unchanged() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("source");
        let skill = source.join("vw/skills/secrets/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        let old = "# Secrets\n`browser_capture_secret` always refuses.\n";
        std::fs::write(&skill, old).unwrap();
        let staged = super::stage(
            &source,
            scratch.path(),
            "Point at the ref to save directly.",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), old);
        assert_eq!(
            std::fs::read_to_string(staged.join("vw/skills/secrets/SKILL.md")).unwrap(),
            "# Secrets\nPoint at the ref to save directly."
        );
    }
}
