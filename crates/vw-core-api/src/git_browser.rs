//! Saved-project Git browser snapshots. Depends on bounded `vw-git` discovery and existing Git selectors.
//! Called by the `GitBrowser` command. Only the existing Git command performs actions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_browser_view::{
    GitBrowserNode, GitBrowserRequest, GitBrowserResult, GitBrowserSnapshot,
};
use crate::git_view::GitResult;

fn note(existing: Option<String>, extra: &str) -> String {
    existing.map_or_else(|| extra.to_owned(), |text| format!("{text} {extra}"))
}

fn watch_paths(root: &Path, discovery: &vw_git::browser::Discovery) -> Vec<PathBuf> {
    let mut paths: BTreeSet<PathBuf> = discovery.directories.iter().cloned().collect();
    let mut child_budget = 256;
    for node in &discovery.nodes {
        for dir in [&node.git_dir, &node.common_dir].into_iter().flatten() {
            for candidate in [
                dir.clone(),
                dir.join("refs"),
                dir.join("refs/heads"),
                dir.join("refs/remotes"),
                dir.join("worktrees"),
            ] {
                if candidate.starts_with(root)
                    && candidate.is_dir()
                    && vw_git::browser::safe_path(Path::new(""), &candidate).is_some()
                {
                    paths.insert(candidate.clone());
                    if child_budget > 0
                        && matches!(
                            candidate.file_name().and_then(|n| n.to_str()),
                            Some("remotes" | "heads" | "worktrees")
                        )
                        && let Ok(children) = std::fs::read_dir(&candidate)
                    {
                        for child in children.take(child_budget.min(32)) {
                            child_budget -= 1;
                            let Ok(child) = child else { continue };
                            let child = child.path();
                            if child.is_dir()
                                && vw_git::browser::safe_path(Path::new(""), &child).is_some()
                            {
                                paths.insert(child);
                            }
                        }
                    }
                }
            }
        }
    }
    paths.into_iter().take(256).collect()
}

// External copies expose only their in-project worktree record, never their folder or Git status.
fn external_branch(node: &vw_git::browser::Node, root: &Path) -> Option<String> {
    if !node.outside || node.kind != "copy" {
        return None;
    }
    let head = node.git_dir.as_ref()?.join("HEAD");
    let head = vw_git::browser::safe_path(Path::new(""), &head)?;
    if !head.starts_with(root) || std::fs::metadata(&head).ok()?.len() > 1024 {
        return None;
    }
    let text = std::fs::read_to_string(head).ok()?;
    text.trim()
        .strip_prefix("ref: refs/heads/")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn link_recorded_copies(
    snapshot: &mut GitBrowserSnapshot,
    records: &[vw_worktrees::records::Record],
    members: &[String],
) {
    for record in records {
        if record.session_deleted
            || record.decision == Some(vw_worktrees::records::Decision::Removed)
            || !members.contains(&record.session_id)
        {
            continue;
        }
        for node in &mut snapshot.nodes {
            if node.outside_project
                && node.kind == "copy"
                && node.branch.as_ref() == Some(&record.branch)
                && node.name == record.folder
                && node.parent_id.as_deref() == record.repo_root.to_str()
            {
                node.session_id = Some(record.session_id.clone());
            }
        }
    }
}

fn link_external_session(snapshot: &mut GitBrowserSnapshot, member: &str, cwd: &Path) {
    for node in &mut snapshot.nodes {
        if node.kind == "copy" && node.outside_project && Path::new(&node.path) == cwd {
            node.session_id = Some(member.to_owned());
        }
    }
}

#[allow(clippy::too_many_lines)]
fn collect(root: &Path, project_id: String) -> (GitBrowserSnapshot, Vec<PathBuf>) {
    let discovery = vw_git::browser::discover(root, vw_git::browser::Limits::default());
    let watch = watch_paths(root, &discovery);
    let parents: BTreeMap<_, _> = discovery
        .nodes
        .iter()
        .filter(|node| node.kind == "repository")
        .filter_map(|node| {
            node.common_dir
                .as_ref()
                .map(|common| (common.clone(), node.path.to_string_lossy().into_owned()))
        })
        .collect();
    let mut nodes = Vec::with_capacity(discovery.nodes.len());
    let status_started = Instant::now();
    let mut status_limited = false;
    for node in discovery.nodes {
        let id = node.path.to_string_lossy().into_owned();
        let mut restricted = node.restricted.clone();
        if node.outside {
            restricted = Some(
                "Vibe Work keeps this copy in its own folder; open its chat to work there.".into(),
            );
        }
        if restricted.is_none() && !vw_git::browser::revalidate(&node, root) {
            restricted =
                Some("The repository folder or metadata changed; refresh before using Git.".into());
        }
        if restricted.is_none() && status_started.elapsed() >= Duration::from_secs(6) {
            restricted = Some("Git status was not read within this refresh budget.".into());
            status_limited = true;
        }
        let status = if restricted.is_none() {
            match vw_git::browser_status::read(&node.path) {
                Ok(status) => Some(status),
                Err(error) => {
                    restricted = Some(format!("Git status is unavailable: {error}"));
                    None
                }
            }
        } else {
            None
        };
        let files = status
            .as_ref()
            .map_or_else(Vec::new, |status| status.files.clone());
        let branch = status
            .as_ref()
            .and_then(|status| status.branch.clone())
            .or_else(|| external_branch(&node, root));
        let parent_id = if node.kind == "copy" {
            node.common_dir
                .as_ref()
                .and_then(|common| parents.get(common).cloned())
        } else if node.kind == "submodule" {
            discovery_parent(&node.path, &parents)
        } else {
            None
        };
        nodes.push(GitBrowserNode {
            id,
            path: node.path.to_string_lossy().into_owned(),
            name: node
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            kind: node.kind,
            parent_id,
            outside_project: node.outside,
            branch,
            status,
            files,
            restricted,
            session_id: None,
            repo: None,
            git_status: None,
        });
    }
    let mut note = discovery.truncated.then(|| {
        "The project tree reached a discovery limit; some folders may be missing.".to_owned()
    });
    if status_limited {
        note = Some(self::note(
            note,
            "Some repository statuses were not read within this refresh budget.",
        ));
    }
    if watch.len() == 256 {
        note = Some(self::note(
            note,
            "Only the first 256 folders can be watched.",
        ));
    }
    (
        GitBrowserSnapshot {
            project_id,
            folder: Some(root.to_string_lossy().into_owned()),
            nodes,
            truncated: discovery.truncated,
            note,
        },
        watch,
    )
}

fn discovery_parent(path: &Path, parents: &BTreeMap<PathBuf, String>) -> Option<String> {
    parents
        .iter()
        .filter(|(_, id)| path.starts_with(id.as_str()) && path != Path::new(id.as_str()))
        .max_by_key(|(_, id)| id.len())
        .map(|(_, id)| id.clone())
}

impl Core {
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn git_browser(
        &self,
        request: GitBrowserRequest,
    ) -> Result<GitBrowserResult, CoreError> {
        let (project_id, watch, stop) = match request {
            GitBrowserRequest::Read { project_id } => (project_id, false, false),
            GitBrowserRequest::Watch { project_id } => (project_id, true, false),
            GitBrowserRequest::Unwatch { project_id } => (project_id, false, true),
        };
        if stop {
            self.shared.git_browser.stop(&project_id);
            return Ok(GitBrowserResult::Stopped {});
        }
        let (folder, members) = self
            .shared
            .projects
            .store
            .read(|store| {
                let project = store.project(&project_id)?;
                let members = store
                    .members()
                    .iter()
                    .filter(|(_, group)| group.as_deref() == Some(project_id.as_str()))
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                Some((project.main_folder.clone(), members))
            })
            .ok_or_else(|| {
                self.shared.git_browser.stop(&project_id);
                CoreError::new(ErrorCode::NotFound, "the saved project was not found")
            })?;
        let Some(folder) = folder else {
            self.shared.git_browser.stop(&project_id);
            return Ok(GitBrowserResult::Snapshot {
                snapshot: GitBrowserSnapshot {
                    project_id,
                    folder: None,
                    nodes: Vec::new(),
                    truncated: false,
                    note: Some(
                        "Choose a main folder for this project to browse its repositories.".into(),
                    ),
                },
            });
        };
        let root = PathBuf::from(&folder);
        if !root.is_absolute() || vw_git::browser::safe_path(Path::new(""), &root).is_none() {
            self.shared.git_browser.stop(&project_id);
            return Err(CoreError::new(
                ErrorCode::Denied,
                "the project's main folder is unavailable or symbolic-linked",
            ));
        }
        let id = project_id.clone();
        let member_scope = root.clone();
        let (mut snapshot, paths) = crate::worktrees::blocking(move || collect(&root, id)).await?;
        if let Ok(records) = self.shared.worktrees.records() {
            let records = crate::worktrees::blocking(move || records.all()).await?;
            link_recorded_copies(&mut snapshot, &records, &members);
        }
        // Action selectors come only from this project's recorded sessions and the Git command's admission.
        let actions_started = Instant::now();
        for member in members {
            if actions_started.elapsed() >= Duration::from_secs(3) {
                snapshot.note = Some(note(
                    snapshot.note,
                    "Git action details reached this refresh's time limit.",
                ));
                break;
            }
            if let Some(cwd) = self
                .shared
                .store
                .view(&member)
                .and_then(|view| crate::changes_facts::facts(&view.state).cwd)
            {
                link_external_session(&mut snapshot, &member, &cwd);
            }
            let in_project = self.shared.store.view(&member).is_some_and(|view| {
                crate::changes_facts::facts(&view.state)
                    .cwd
                    .as_ref()
                    .is_some_and(|cwd| {
                        cwd.starts_with(&member_scope)
                            && vw_git::browser::safe_path(Path::new(""), cwd).as_deref()
                                == Some(cwd.as_path())
                            && snapshot.nodes.iter().any(|node| {
                                node.restricted.is_none() && cwd.starts_with(Path::new(&node.path))
                            })
                    })
            });
            if !in_project {
                continue;
            }
            let Ok(repos) = self.git_repos(&member) else {
                continue;
            };
            for (selector, place) in repos {
                if actions_started.elapsed() >= Duration::from_secs(3) {
                    break;
                }
                let Some(node) = snapshot
                    .nodes
                    .iter_mut()
                    .find(|node| node.restricted.is_none() && Path::new(&node.path) == place.root)
                else {
                    continue;
                };
                if node.kind == "submodule"
                    || (node.session_id.is_some()
                        && node
                            .git_status
                            .as_ref()
                            .is_none_or(|status| !status.running))
                {
                    continue;
                }
                let Ok(GitResult::Status { status }) =
                    self.git_status(&member, Some(&selector.root)).await
                else {
                    continue;
                };
                if !status.available {
                    continue;
                }
                if node.session_id.is_some() && status.running {
                    continue;
                }
                node.session_id = Some(member.clone());
                node.repo = Some(selector.root);
                node.git_status = Some(status);
            }
        }
        if (watch || self.shared.git_browser.contains(&project_id))
            && let Some(problem) = self
                .shared
                .git_browser
                .start(&self.shared, &project_id, paths)
        {
            snapshot.note = Some(note(snapshot.note, &problem));
        }
        Ok(GitBrowserResult::Snapshot { snapshot })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_metadata_never_becomes_a_watch_or_action_target() {
        let root = std::env::temp_dir().join(format!("vw-git-browser-core-{}", std::process::id()));
        let project = root.join("project");
        let outside = root.join("outside");
        assert!(std::fs::create_dir_all(&project).is_ok());
        assert!(std::fs::create_dir_all(&outside).is_ok());
        assert!(
            std::fs::write(
                project.join(".git"),
                format!("gitdir: {}\n", outside.display())
            )
            .is_ok()
        );
        let (snapshot, watches) = collect(&project, "saved".into());
        assert_eq!(snapshot.nodes.len(), 1);
        let node = &snapshot.nodes[0];
        assert!(node.restricted.is_some());
        assert!(node.status.is_none());
        assert!(node.session_id.is_none());
        assert!(watches.iter().all(|path| path.starts_with(&project)));
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn external_copy_keeps_branch_without_status_or_git_actions() {
        let root = std::env::temp_dir().join(format!("vw-browser-copy-{}", std::process::id()));
        let project = root.join("project");
        let outside = root.join("copy");
        let record = project.join(".git/worktrees/copy");
        std::fs::create_dir_all(&record).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join(".git"), "not read outside scope").unwrap();
        std::fs::write(
            record.join("gitdir"),
            outside.join(".git").to_string_lossy().as_bytes(),
        )
        .unwrap();
        std::fs::write(record.join("HEAD"), "ref: refs/heads/vibe/real\n").unwrap();
        let (mut snapshot, watches) = collect(&project, "saved".into());
        let record = vw_worktrees::records::Record {
            session_id: "copy-session".into(),
            repo_root: project.clone(),
            base_name: "main".into(),
            base_commit: "a".repeat(40),
            branch: "vibe/real".into(),
            folder: "copy".into(),
            created_branch: true,
            created_at_ms: 0,
            decision: None,
            session_deleted: false,
            session_archived: false,
        };
        link_recorded_copies(&mut snapshot, &[record], &["copy-session".into()]);
        let copy = snapshot.nodes.iter().find(|n| n.outside_project).unwrap();
        assert_eq!(copy.branch.as_deref(), Some("vibe/real"));
        assert_eq!(copy.session_id.as_deref(), Some("copy-session"));
        assert_eq!(
            copy.restricted.as_deref(),
            Some("Vibe Work keeps this copy in its own folder; open its chat to work there.")
        );
        assert!(copy.status.is_none());
        assert!(copy.repo.is_none());
        assert!(copy.git_status.is_none());
        assert!(watches.iter().all(|path| path.starts_with(&project)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn submodule_parent_uses_nearest_repository() {
        let parents = BTreeMap::from([
            (PathBuf::from("/p/.git"), "/p".into()),
            (PathBuf::from("/p/nested/.git"), "/p/nested".into()),
        ]);
        assert_eq!(
            discovery_parent(Path::new("/p/nested/lib"), &parents),
            Some("/p/nested".into())
        );
        assert_eq!(discovery_parent(Path::new("/elsewhere"), &parents), None);
    }
}
