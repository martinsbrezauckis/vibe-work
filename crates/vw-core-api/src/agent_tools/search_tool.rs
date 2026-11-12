//! Session-credential search adapter. Uses the Files search engine and the read folder guard;
//! app state and the vault are excluded even when a chat's folder contains them.
use crate::core::Shared;
use crate::read_exclusions::{ReadExclusions, under};
use crate::search_agent::{Request, run};
use crate::search_files::Folder;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vw_agent_tools::folders::{Access, Layout, resolve};

pub(super) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    args: Value,
) -> Result<Value, String> {
    let request: Request =
        serde_json::from_value(args).map_err(|_| "Invalid search arguments".to_owned())?;
    let mut roots = super::session::folders(shared, session)?;
    let mut seen = std::collections::BTreeSet::new();
    roots.retain(|path| seen.insert(vw_procmon::normalize(path)));
    let shared = shared.clone();
    let session = session.to_owned();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        let layout =
            Layout::new(&shared.instructions.own_home, &roots[0]).map_err(|e| e.to_string())?;
        let exclusions = ReadExclusions::for_chat(&shared, &roots[0])?;
        let forbidden = |path: &Path| exclusions.excludes(path);
        // A protected folder is never inspected for project configuration either.
        if forbidden(&roots[0]) {
            return Ok(Value::String("No hits.".into()));
        }
        let policy_roots: Vec<_> = roots
            .iter()
            .filter(|path| !forbidden(path))
            .cloned()
            .collect();
        let rules = crate::search_read_rules::ReadRules::load(
            &shared.instructions.own_home,
            &policy_roots,
            deadline,
        )?;
        let folders: Vec<_> = roots
            .iter()
            .map(|p| Folder {
                path: p.clone(),
                session_id: None,
            })
            .collect();
        let allowed = |path: &Path| {
            if forbidden(path) {
                return false;
            }
            let Ok(real) = resolve(path) else {
                return false;
            };
            if !rules.allows(&real) || forbidden(&real) || !roots.iter().any(|r| under(&real, r)) {
                return false;
            }
            layout
                .check(path, &roots[0], Access::Read, &|copy, own| {
                    let tools = shared.agent_tools.as_ref().ok_or("Tools unavailable")?;
                    super::copycheck::decide(&shared, &tools.copy_locks, &session, copy, own)
                })
                .is_none()
        };
        run(
            &folders,
            &request,
            &allowed,
            &|file, path| exclusions.check_opened(file, path, Some(deadline)).is_ok(),
            deadline.saturating_duration_since(Instant::now()),
        )
        .map(Value::String)
    })
    .await
    .map_err(|e| format!("Search failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn credential_search_uses_only_its_cwd_and_workspace_roots() {
        let f = crate::feature_test_core::Fixture::new().await;
        let a = f.root.join("a");
        let b = f.root.join("b");
        let extra = f.root.join("extra");
        for p in [&a, &b, &extra] {
            std::fs::create_dir_all(p).unwrap();
            std::fs::write(p.join("hit.txt"), p.file_name().unwrap().to_str().unwrap()).unwrap();
        }
        f.load("a", &a, std::slice::from_ref(&extra), false);
        f.load("b", &b, &[], false);
        let answer = call(
            &f.core.shared,
            "a",
            serde_json::json!({"query":".","regex":true}),
        )
        .await
        .unwrap();
        let text = answer.as_str().unwrap();
        assert!(text.contains("hit.txt:1: a"));
        assert!(text.contains("hit.txt:1: extra"));
        assert!(!text.contains("hit.txt:1: b"));
        assert!(
            call(
                &f.core.shared,
                "missing",
                serde_json::json!({"glob":"**/*"})
            )
            .await
            .is_err()
        );
        assert!(
            call(
                &f.core.shared,
                "a",
                serde_json::json!({"glob":"**/*","folder":b})
            )
            .await
            .is_err()
        );
        f.finish().await;
    }
    #[tokio::test]
    async fn a_broad_chat_root_cannot_search_app_state_and_a_copy_cannot_search_another_copy() {
        let f = crate::feature_test_core::Fixture::new().await;
        std::fs::write(f.root.join("ok.txt"), "needle").unwrap();
        std::fs::write(f.root.join("app-data/state.txt"), "needle").unwrap();
        let private =
            vw_vault::private_location::private_vault_dir(&f.root.join("app-data")).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("key.txt"), "needle").unwrap();
        f.load("broad", &f.root, &[], false);
        assert_eq!(
            call(
                &f.core.shared,
                "broad",
                serde_json::json!({"query":"needle"})
            )
            .await
            .unwrap(),
            "ok.txt:1: needle"
        );
        // Joined per component: one "a/b" string keeps its forward slashes on Windows, where the tool prints `\`.
        let own = f
            .root
            .join("app-data")
            .join("vibe")
            .join("worktrees")
            .join("repo")
            .join("own");
        let other = own.with_file_name("other");
        f.load("own", &own, std::slice::from_ref(&other), false);
        f.load("other", &other, &[], true);
        std::fs::write(own.join("own.txt"), "needle").unwrap();
        std::fs::write(other.join("other.txt"), "needle").unwrap();
        assert_eq!(
            call(&f.core.shared, "own", serde_json::json!({"query":"needle"}))
                .await
                .unwrap(),
            format!("Folder: {}\nown.txt:1: needle", own.display())
        );
        f.finish().await;
    }
    #[tokio::test]
    async fn search_travels_through_the_authenticated_session_wire_and_single_use_receipt() {
        let f = crate::feature_test_core::Fixture::new().await;
        let project = f.root.join("project");
        f.load("a", &project, &[], false);
        std::fs::write(project.join("hit.txt"), "needle").unwrap();
        let tools = f.core.shared.agent_tools.clone().unwrap();
        let credential = tools.table.for_session("a").unwrap();
        f.core.shared.access.observe_runtime(
            "a",
            &serde_json::json!({"activeAgent":{"name":"auto-approve"}}),
        );
        let args=crate::ask::tool_calls::receipts::issue(&f.core.shared,&serde_json::json!({"session_id":"a","tool_call_id":"search-wire","tool_name":"mcp_vw.search","tool_input":{"query":"needle"}})).await.unwrap();
        let replay = args.clone();
        assert!(
            super::super::tools::call(&f.core.shared, &tools, "b", "search", replay.clone())
                .await
                .is_err()
        );
        let endpoint = tools.endpoint.to_string();
        let answer = tokio::task::spawn_blocking(move || {
            vw_agent_tools::wire::call(
                &endpoint,
                &credential,
                "search",
                args,
                Duration::from_secs(10),
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(answer, "hit.txt:1: needle");
        assert!(
            super::super::tools::call(&f.core.shared, &tools, "a", "search", replay)
                .await
                .is_err()
        );
        f.finish().await;
    }
    #[tokio::test]
    async fn a_vault_chat_root_is_excluded_before_project_policy_is_read() {
        let f = crate::feature_test_core::Fixture::new().await;
        let vault =
            vw_vault::private_location::private_vault_dir(&f.root.join("app-data")).unwrap();
        std::fs::create_dir_all(vault.join(".vibe")).unwrap();
        std::fs::write(vault.join(".vibe/config.toml"), "[").unwrap();
        f.load("vault", &vault, &[], false);
        assert_eq!(
            call(&f.core.shared, "vault", serde_json::json!({"glob":"**/*"}))
                .await
                .unwrap(),
            "No hits."
        );
        f.finish().await;
    }
}
