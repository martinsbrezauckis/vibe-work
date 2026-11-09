//! Session-authorized video extraction. Pins a regular local file, checks model/image allowance and cancels
//! on disconnect, Stop, session deletion or credential revocation; the helper never receives a session secret.
use super::AgentTools;
use crate::core::Shared;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use vw_agent_tools::{
    folders::{Access, Layout},
    video::{Output, Request},
};
pub(super) fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call_id: &str,
    args: Value,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    vw_agent_tools::video::require_available()?;
    let request: Request =
        serde_json::from_value(args.clone()).map_err(|e| format!("Invalid video request: {e}. Supply integer millisecond times, a supported selection and output mode."))?;
    let frames = request.frame_limit()?;
    crate::ask::tool_calls::changed(shared, owner);
    let (shared, owner, source, call_id) = (
        shared.clone(),
        owner.to_owned(),
        source.to_owned(),
        call_id.to_owned(),
    );
    Ok(Box::pin(async move {
        let tools = shared.agent_tools.as_ref().ok_or("Tools unavailable.")?;
        let result = call(&shared, tools, &owner, args).await;
        let result = match result {
            Ok(mut reply) if reply["image_count"] == 1 => {
                reply["source"] =
                    serde_json::json!(request.path.rsplit(['/', '\\']).next().unwrap_or("Video"));
                reply["output"] =
                    serde_json::to_value(request.output).map_err(|e| e.to_string())?;
                reply["sheet_edge"] = serde_json::json!(
                    request
                        .max_edge
                        .unwrap_or(vw_preview::MAX_WIDTH)
                        .min(vw_preview::MAX_WIDTH)
                );
                save(&shared, &owner, &source, &call_id, reply.clone()).await?;
                Ok(reply)
            }
            Ok(mut reply) => {
                reply["source"] =
                    serde_json::json!(request.path.rsplit(['/', '\\']).next().unwrap_or("Video"));
                tools
                    .video_history
                    .outcome(&source, &source, &call_id, reply.clone())?;
                Ok(reply)
            }
            Err(reason) => {
                let (row, agent) = crate::video_projection::failure(&reason);
                let meta = serde_json::json!({"planned_frames":frames,"image_count":usize::from(request.output!=Output::Metadata),"source":request.path.rsplit(['/', '\\']).next().unwrap_or("Video"),"error":row});
                tools
                    .video_history
                    .outcome(&source, &source, &call_id, meta)?;
                Err(agent)
            }
        };
        crate::ask::tool_calls::changed(&shared, &owner);
        result
    }))
}
async fn save(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call_id: &str,
    reply: Value,
) -> Result<(), String> {
    let tools = shared.agent_tools.clone().ok_or("Tools unavailable.")?;
    let cwd = shared
        .names
        .cwd_of(&shared.store, owner)
        .ok_or("The session folder is unavailable.")?;
    let (source, call_id) = (source.to_owned(), call_id.to_owned());
    tokio::task::spawn_blocking(move || {
        tools
            .video_history
            .save(Path::new(&cwd), &source, &source, &call_id, reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(super) async fn call(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    owner: &str,
    args: Value,
) -> Result<Value, String> {
    vw_agent_tools::video::require_available()?;
    let request: Request =
        serde_json::from_value(args).map_err(|e| format!("decode_failed: invalid video request: {e}. Supply integer millisecond times, a supported selection and output mode."))?;
    request.times()?;
    let view = shared.store.view(owner).ok_or("cancelled: chat closed")?;
    let image = request.output != Output::Metadata;
    if image
        && shared.store.runtime(owner).and_then(|r| {
            r.pointer("/config/activeModel/supportsImages")
                .and_then(Value::as_bool)
        }) != Some(true)
    {
        return Err("unsupported_codec: the selected model has no confirmed image support".into());
    }
    let mut reservation = tools.video.reserve(owner, &view.state, image)?;
    let stop = crate::stops::generation(shared, owner);
    let helper = tools.config.binary.with_file_name(if cfg!(windows) {
        "vw-video-helper.exe"
    } else {
        "vw-video-helper"
    });
    let core = shared.clone();
    let session = owner.to_owned();
    let path = request.path.clone();
    let extraction = async {
        let input = tokio::task::spawn_blocking(move || snapshot(&core, &session, &path))
            .await
            .map_err(|_| "decode_failed: input preparation")??;
        let id = vw_agent_tools::credential::new_credential()
            .map_err(|_| "backend_unavailable: job identity")?;
        let reply =
            vw_video::sandbox::extract(&helper, input, vw_video::protocol::Job { id, request })
                .await?;
        Ok::<_, String>(reply.into_tool())
    };
    let invalidated = async {
        loop {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if crate::stops::generation(shared, owner) != stop
                || !tools.table.has_session(owner)
                || shared.store.view(owner).is_none()
            {
                break;
            }
        }
    };
    let reply = tokio::select! {biased; ()=invalidated=>return Err("cancelled: video job ended".into()), result=tokio::time::timeout(Duration::from_secs(40),extraction)=>result.map_err(|_|"timeout: video job deadline")??};
    if crate::stops::generation(shared, owner) != stop || !tools.table.has_session(owner) {
        return Err("cancelled: video job ended".into());
    }
    let bytes = serde_json::to_vec(&reply).map_err(|_| "decode_failed: image envelope")?;
    if bytes.len() + 32 >= vw_agent_tools::wire::MAX_ANSWER {
        return Err("limit_exceeded: image envelope bytes".into());
    }
    reservation.sent(
        &shared
            .store
            .view(owner)
            .ok_or("cancelled: chat closed")?
            .state,
        reply["image_count"] == 1,
    )?;
    Ok(reply)
}
fn snapshot(shared: &Shared, owner: &str, path: &str) -> Result<vw_video::input::Input, String> {
    let roots = super::session::folders(shared, owner)?;
    let given = path;
    #[cfg(windows)]
    let native = vw_agent_tools::video::windows_path(path);
    #[cfg(windows)]
    let path = native.as_ref();
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        roots[0].join(path)
    };
    let real = candidate
        .canonicalize()
        .map_err(|error| input_error(given, &error))?;
    let layout = Layout::new(&shared.instructions.own_home, &roots[0])
        .map_err(|_| "decode_failed: folder policy")?;
    let exclusions = crate::read_exclusions::ReadExclusions::for_chat(shared, &roots[0])?;
    if exclusions.excludes(&real) {
        return Err("decode_failed: protected source location".into());
    }
    let rules = crate::search_read_rules::ReadRules::load(
        &shared.instructions.own_home,
        &roots,
        Instant::now() + Duration::from_secs(4),
    )?;
    if !rules.allows(&real) {
        return Err("decode_failed: file read policy denies this video".into());
    }
    if let Some(reason) = layout.check(&real, &roots[0], Access::Read, &|copy, own| {
        let tools = shared.agent_tools.as_ref().ok_or("Tools unavailable")?;
        super::copycheck::decide(shared, &tools.copy_locks, owner, copy, own)
    }) {
        return Err(reason);
    }
    #[cfg(not(unix))]
    let parent = candidate.parent().ok_or("decode_failed: input parent")?;
    #[cfg(not(unix))]
    let name = candidate
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("decode_failed: input filename")?;
    #[cfg(unix)]
    let file = {
        // No read occurs until the final handle path matches the already authorized canonical path.
        // NONBLOCK also prevents a regular-file-to-FIFO race from holding the preparation thread.
        for part in candidate.ancestors() {
            if part
                .symlink_metadata()
                .map_err(|_| "changed_input: input path")?
                .file_type()
                .is_symlink()
            {
                return Err("decode_failed: input links are unsupported".into());
            }
        }
        let fd = rustix::fs::open(
            &candidate,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::NOCTTY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| input_error(given, &error.into()))?;
        std::fs::File::from(fd)
    };
    #[cfg(not(unix))]
    let file = crate::search_walk::open(parent, name)
        .map_err(|error| input_error(given, &error))?
        .into_std();
    if !file
        .metadata()
        .map_err(|error| input_error(given, &error))?
        .is_file()
    {
        return Err(format!("cannot_read: {given}"));
    }
    exclusions.check_opened(&file, &real, None)?;
    vw_video::input::Input::copy(file, &real)
}

fn input_error(path: &str, error: &std::io::Error) -> String {
    let code = if error.kind() == std::io::ErrorKind::NotFound {
        "not_found"
    } else {
        "cannot_read"
    };
    format!("{code}: {path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde_json::json;
    #[tokio::test]
    async fn video_snapshot_names_missing_and_unreadable_sources_without_a_decode_diagnosis() {
        let f = crate::feature_test_core::Fixture::new().await;
        let cwd = f.root.join("project");
        f.load("video", &cwd, &[], true);
        let given = "my ārā clip.mp4";
        let reason = snapshot(&f.core.shared, "video", given).err().unwrap();
        assert_eq!(reason, format!("not_found: {given}"));
        let (row, agent) = crate::video_projection::failure(&reason);
        assert_eq!(row, format!("Not found: {given}."));
        assert!(agent.contains("Check the file name and folder."), "{agent}");
        assert!(!agent.contains("decode"), "{agent}");
        let folder = cwd.join("folder.mp4");
        std::fs::create_dir(&folder).unwrap();
        let reason = snapshot(&f.core.shared, "video", "folder.mp4")
            .err()
            .unwrap();
        let (row, agent) = crate::video_projection::failure(&reason);
        assert_eq!(row, "Cannot read: folder.mp4.");
        assert!(agent.contains("readable local file"), "{agent}");
        assert!(!agent.contains("decode"), "{agent}");
        f.finish().await;
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn git_bash_video_paths_keep_the_same_source_and_read_policy() {
        let f = crate::feature_test_core::Fixture::new().await;
        let cwd = f.root.join("project");
        f.load("video", &cwd, &[], true);
        let clip = cwd.join("video clip.mp4");
        std::fs::write(&clip, b"ordinary video input").unwrap();
        let msys = |path: &Path| {
            let native = path.canonicalize().unwrap().to_string_lossy().into_owned();
            let plain = native.strip_prefix(r"\\?\").unwrap_or(&native);
            format!(
                "/{}{}",
                plain[..1].to_ascii_lowercase(),
                plain[2..].replace('\\', "/")
            )
        };
        let input = snapshot(&f.core.shared, "video", &msys(&clip)).unwrap();
        assert_eq!(
            std::fs::read(input.path()).unwrap(),
            b"ordinary video input"
        );
        let protected = f.root.join("app-data/private.bin");
        std::fs::write(&protected, b"private bytes").unwrap();
        assert!(snapshot(&f.core.shared, "video", &msys(&protected)).is_err());
        f.finish().await;
    }
    #[cfg(all(target_os = "linux", not(feature = "linux-video")))]
    #[tokio::test]
    async fn linux_video_refuses_before_request_validation_or_file_access() {
        let f = crate::feature_test_core::Fixture::new().await;
        let tools = f.core.shared.agent_tools.as_ref().unwrap();
        let reason = call(&f.core.shared, tools, "missing-chat", Value::Null)
            .await
            .unwrap_err();
        assert!(reason.starts_with("Not run: video review is not available on Linux yet."));
        assert!(reason.contains("Ask for still images"));
        let prepared = prepare(
            &f.core.shared,
            "missing-chat",
            "source",
            "call",
            Value::Null,
        );
        assert_eq!(prepared.err(), Some(reason.clone()));
        let direct = super::super::tools::call(
            &f.core.shared,
            tools,
            "missing-chat",
            "video_frames",
            Value::Null,
        )
        .await;
        assert_eq!(direct, Err(reason));
        // A stale cached tool must not open an approval control in Ask mode.
        let permission = crate::ask::tool_calls::permission(
            &f.core.shared,
            "missing-chat",
            "source",
            "call",
            "video_frames",
            &Value::Null,
        )
        .await;
        assert_eq!(permission, Ok(()));
        assert_eq!(f.core.asks().len(), 0);
        f.finish().await;
    }
    #[tokio::test]
    async fn video_snapshot_refuses_a_hard_link_to_app_state() {
        let f = crate::feature_test_core::Fixture::new().await;
        let cwd = f.root.join("project");
        f.load("video", &cwd, &[], true);
        let protected = f.root.join("app-data/private.bin");
        let alias = cwd.join("clip.webm");
        std::fs::write(&protected, b"private bytes").unwrap();
        std::fs::hard_link(&protected, &alias).unwrap();
        let result = snapshot(&f.core.shared, "video", alias.to_str().unwrap());
        assert!(
            result.is_err(),
            "video copied a hard link to protected app state"
        );
        std::fs::remove_file(&alias).unwrap();
        std::fs::write(&alias, b"ordinary video input").unwrap();
        assert!(snapshot(&f.core.shared, "video", alias.to_str().unwrap()).is_ok());
        f.finish().await;
    }
    #[tokio::test]
    async fn child_frames_belong_to_the_displaying_chat_and_its_deletion() {
        let f = crate::feature_test_core::Fixture::new().await;
        let cwd = f.root.join("project");
        f.load("parent", &cwd, &[], true);
        f.load("child", &cwd, &[], true);
        let reply = json!({"video_id":"job","planned_frames":1,"image_count":1,"output":"frame",
            "image_size":[1,1],"columns":1,"frames":[{"index":0,"frame_id":0,"actual_ns":0}],
            vw_agent_tools::wire::IMAGE_KEY:{"data":base64::engine::general_purpose::STANDARD.encode([0])}});
        save(&f.core.shared, "parent", "child", "call", reply)
            .await
            .unwrap();
        let tools = f.core.shared.agent_tools.as_ref().unwrap();
        assert!(tools.video_history.read("child", "video:job:0").is_ok());
        assert!(tools.video_history.read("parent", "video:job:0").is_err());
        tools.session_deleted("child");
        assert!(tools.video_history.read("child", "video:job:0").is_err());
        assert_eq!(
            std::fs::read_dir(cwd.join("attachments/child"))
                .unwrap()
                .count(),
            0
        );
        f.finish().await;
    }
}
