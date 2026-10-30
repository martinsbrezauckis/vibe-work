//! Where a skill copy's `SKILL.md` lives and when it last changed, for the Skills tab's time. Vibe returns no file
//! paths, so the core looks in the folders Vibe itself searches (Vibe 3.0.3 `core/skills/manager.py`,
//! `harness_files/_harness_manager.py`), in Vibe's order:
//! - a project copy: `<folder>/.vibe/skills`, then `<folder>/.agents/skills` (the trusted chat folder is Vibe's project
//!   root);
//! - a global local copy: `VIBE_HOME/skills`, then `AGENTS_HOME/skills` (`$AGENTS_HOME`, else `~/.agents`);
//! - a registry copy: `VIBE_HOME/skills-registry-cache/store/<skill id>/<version>/SKILL.md`.
//!
//! A local copy is the first `<dir>/<child>/SKILL.md` whose front-matter `name` (else the child folder's name) is the
//! copy's name, as Vibe matches it. Built-in and plugin copies have no time here, and neither do skills from Vibe's
//! `skill_paths` setting (Vibe Work never writes it).
//! Depends on: vw-skills (copy identity), `std::fs`. Called by: `skills_rows` (the list view).
//! Invariants: read only (a few `stat`s and the first 4 KiB of candidate files); only regular files are opened or
//! timed, and an open never waits (a FIFO named `SKILL.md` cannot block the read); a registry id that is not one plain
//! path segment is never joined into a path; anything not found gives `None`, never an error. The caller runs it off
//! the async workers (`spawn_blocking`).
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use vw_skills::{SkillScope, SkillSource, SkillSummary};

/// The folders searched for one Skills read.
pub(crate) struct SkillPlaces {
    project: [PathBuf; 2],
    global: Vec<PathBuf>,
    store: PathBuf,
}

impl SkillPlaces {
    /// The places for Vibe Work's own `vibe_home` and the chat folder `folder` the list was read in.
    pub(crate) fn new(vibe_home: &Path, folder: &Path) -> Self {
        let agents_home = std::env::var_os("AGENTS_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .filter(|v| !v.is_empty())
                    .map(|home| PathBuf::from(home).join(".agents"))
            });
        Self {
            project: [
                folder.join(".vibe").join("skills"),
                folder.join(".agents").join("skills"),
            ],
            global: std::iter::once(vibe_home.join("skills"))
                .chain(agents_home.map(|home| home.join("skills")))
                .collect(),
            store: vibe_home.join("skills-registry-cache").join("store"),
        }
    }

    /// When `copy`'s skill file last changed, in milliseconds since the epoch.
    pub(crate) fn modified_ms(&self, copy: &SkillSummary) -> Option<i64> {
        let file = match (copy.source, &copy.registry) {
            // A registry copy without its identity is not one this lookup can place.
            (SkillSource::Builtin | SkillSource::Plugin, _) | (SkillSource::Registry, None) => {
                return None;
            }
            (_, Some(registry)) => {
                let id = Path::new(&registry.skill_id);
                let single = id.components().count() == 1
                    && matches!(id.components().next(), Some(Component::Normal(_)));
                if !single {
                    return None;
                }
                self.store
                    .join(id)
                    .join(registry.version.to_string())
                    .join("SKILL.md")
            }
            (SkillSource::Local, None) => {
                let dirs: &[PathBuf] = match copy.scope {
                    SkillScope::Builtin => return None,
                    SkillScope::Project => &self.project,
                    SkillScope::Global => &self.global,
                };
                dirs.iter().find_map(|dir| local_file(dir, &copy.name))?
            }
        };
        let metadata = std::fs::metadata(file)
            .ok()
            .filter(std::fs::Metadata::is_file)?;
        let modified = metadata.modified().ok()?;
        i64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
    }
}

/// The first `<dir>/<child>/SKILL.md` that names `name` (its own folder first, the usual layout).
fn local_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let own = dir.join(name).join("SKILL.md");
    if Path::new(name).components().count() == 1 && named(&own, name) {
        return Some(own);
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path().join("SKILL.md")))
        .collect();
    // Vibe walks `iterdir()` order, which the platform decides; sorted keeps the pick stable.
    children.sort();
    children.into_iter().find(|file| named(file, name))
}

/// Whether the file at `file` is a skill named `name`: its front matter's `name`, else its folder's name.
fn named(file: &Path, name: &str) -> bool {
    let Some(handle) = open_regular(file) else {
        return false;
    };
    let mut head = String::new();
    if handle.take(4096).read_to_string(&mut head).is_err() {
        return false;
    }
    front_matter_name(&head).map_or_else(
        || {
            file.parent()
                .and_then(Path::file_name)
                .is_some_and(|f| f == name)
        },
        |declared| declared == name,
    )
}

/// `file` opened for reading when it is a regular file, else `None`. A FIFO or device named `SKILL.md` would block a
/// plain open (a FIFO until a writer appears), so on unix the open never waits (`O_NONBLOCK`) and the type is checked
/// on the opened handle, not by an earlier `stat` that could race with a swap.
fn open_regular(file: &Path) -> Option<std::fs::File> {
    #[cfg(unix)]
    let handle = {
        use rustix::fs::{Mode, OFlags};
        let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
        std::fs::File::from(rustix::fs::open(file, flags, Mode::empty()).ok()?)
    };
    #[cfg(not(unix))]
    let handle = std::fs::File::open(file).ok()?;
    handle.metadata().ok()?.is_file().then_some(handle)
}

/// The `name:` value of a leading `---` front-matter block, quotes removed.
fn front_matter_name(text: &str) -> Option<&str> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    lines
        .take_while(|line| line.trim_end() != "---")
        .find_map(|line| line.strip_prefix("name:"))
        .map(|value| value.trim().trim_matches(|c| c == '"' || c == '\''))
}
