//! Agent proposal validation before capture: bounded text inside the explicitly allowed Vibe folders.
//! Uses filesystem metadata without following links; called only by the authenticated tool handler.
use serde::Deserialize;
use std::path::{Component, Path};
use vw_proposals::{Change, Proposal, Request};
const MAX_FILES: usize = 20;
const MAX_BYTES: u64 = 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    reason: String,
    changes: Vec<FileInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    path: String,
    kind: String,
    content: Option<String>,
}

pub(super) fn capture(
    root: &Path,
    session_id: &str,
    args: serde_json::Value,
) -> Result<Proposal, String> {
    let input: Input =
        serde_json::from_value(args).map_err(|e| format!("invalid proposal: {e}"))?;
    if input.changes.is_empty() || input.changes.len() > MAX_FILES {
        return Err("a proposal must contain 1 to 20 files".into());
    }
    let mut changes = Vec::new();
    for file in input.changes {
        let path = Path::new(&file.path);
        let parts: Vec<_> = path.components().collect();
        let allowed = file.path == "config.toml"
            || (parts.len() > 1
                && matches!(parts.first(),Some(Component::Normal(n)) if *n == "skills" || *n == "agents" || *n == "prompts"));
        if !allowed
            || file.path.contains('\\')
            || file.path.split('/').any(|p| p == "..")
            || parts.iter().any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(format!(
                "{} is outside the allowed area. Use a path relative to Vibe Work's Vibe home ({}), such as skills/<name>/SKILL.md. Allowed: skills/**, agents/**, prompts/**, config.toml.",
                file.path,
                root.display()
            ));
        }
        let mut target = root.to_owned();
        for part in &parts {
            target.push(part);
            match std::fs::symlink_metadata(&target) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!("symlink in proposal path: {}", file.path));
                }
                Ok(m) if target == root.join(path) && m.len() > MAX_BYTES => {
                    return Err(format!("{} is over 1 MB", file.path));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("cannot inspect {}: {e}", file.path)),
            }
        }
        let content = || -> Result<Vec<u8>, String> {
            let text = file
                .content
                .as_ref()
                .ok_or("content is required for add/edit")?;
            if text.len() as u64 > MAX_BYTES {
                return Err(format!("{} is over 1 MB", file.path));
            }
            if text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return Err(format!(
                    "binary content is not allowed for add/edit: {}",
                    file.path
                ));
            }
            Ok(text.as_bytes().to_vec())
        };
        changes.push(match file.kind.as_str() {
            "add" => Change::Add {
                path: path.into(),
                content: content()?,
            },
            "edit" => Change::Edit {
                path: path.into(),
                content: content()?,
            },
            "remove" if file.content.is_none() => Change::Remove { path: path.into() },
            _ => {
                return Err(
                    "kind must be add/edit (with content), or remove (without content)".into(),
                );
            }
        });
    }
    let proposal = Proposal::with_new_directories(
        root,
        Request {
            session_id: session_id.into(),
            reason: input.reason,
            changes,
        },
    )
    .map_err(|e| e.to_string())?;
    let files = proposal.review();
    if files
        .iter()
        .any(|f| f.before_bytes as u64 > MAX_BYTES || f.after_bytes as u64 > MAX_BYTES)
    {
        return Err("a file is over 1 MB".into());
    }
    if files.iter().any(|f| {
        f.kind != vw_proposals::Kind::Remove && f.content_kind == vw_proposals::ContentKind::Binary
    }) {
        return Err("binary content is not allowed for add/edit".into());
    }
    Ok(proposal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p3b_invalid_path_explains_the_protected_root_and_relative_example() {
        let root = Path::new("/synthetic/vibe-work/vibe-home");
        let error = capture(root, "session", serde_json::json!({
            "reason":"Add a skill", "changes":[{"path":"~/.vibe/skills/demo/SKILL.md", "kind":"add", "content":"demo"}]
        })).err().unwrap();
        assert!(error.contains(&root.display().to_string()), "{error}");
        assert!(
            error.contains("relative") && error.contains("skills/<name>/SKILL.md"),
            "{error}"
        );
    }
}
