//! Search cannot bypass file-read denials. Reads bounded persisted Vibe policy layers and conservatively
//! unions their `read_file` denials, including profile and cached administrator rules; never returns config text.
use globset::{GlobBuilder, GlobMatcher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;
use vw_agent_tools::folders::resolve;

#[derive(Default)]
pub(crate) struct ReadRules {
    never: bool,
    patterns: Vec<GlobMatcher>,
}
impl ReadRules {
    pub(crate) fn load(home: &Path, roots: &[PathBuf], deadline: Instant) -> Result<Self, String> {
        let mut rules = Self::default();
        rules.config(&home.join("config.toml"))?;
        rules.profiles(&home.join("agents"), deadline)?;
        for root in roots {
            for ancestor in root.ancestors().take(64) {
                if Instant::now() >= deadline {
                    return Err("Search limit reached.".into());
                }
                rules.config(&ancestor.join(".vibe/config.toml"))?;
                rules.profiles(&ancestor.join(".vibe/agents"), deadline)?;
                if Some(ancestor) == home.parent() {
                    break;
                }
            }
        }
        if let Some(text) = read(&home.join("managed_config_cache.json"))? {
            let cache: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| "Cannot check file read rules.")?;
            for entry in cache.as_object().into_iter().flat_map(|m| m.values()) {
                if let Some(text) = entry["payload"].as_str() {
                    rules.parse(text)?;
                }
            }
        }
        // Environment overrides are read as policy only; they never become search output.
        if let Ok(text) = std::env::var("VIBE_TOOLS") {
            let tools: toml::Table =
                serde_json::from_str(&text).map_err(|_| "Cannot check file read rules.")?;
            rules.table(&tools)?;
        }
        Ok(rules)
    }
    fn profiles(&mut self, dir: &Path, deadline: Instant) -> Result<(), String> {
        let directory = match crate::search_walk::root(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("Cannot check file read rules.".into()),
        };
        let entries = directory
            .entries()
            .map_err(|_| "Cannot check file read rules.")?;
        for (index, entry) in entries.enumerate() {
            if index >= 256 || Instant::now() >= deadline {
                return Err("Search limit reached.".into());
            }
            let entry = entry.map_err(|_| "Cannot check file read rules.")?;
            let path = dir.join(entry.file_name());
            if path.extension().is_some_and(|e| e == "toml") {
                self.config(&path)?;
            }
        }
        Ok(())
    }
    fn config(&mut self, path: &Path) -> Result<(), String> {
        if let Some(text) = read(path)? {
            self.parse(&text)?;
        }
        Ok(())
    }
    fn parse(&mut self, text: &str) -> Result<(), String> {
        let table: toml::Table = text.parse().map_err(|_| "Cannot check file read rules.")?;
        if let Some(tools) = table.get("tools").and_then(toml::Value::as_table) {
            self.table(tools)?;
        }
        if table
            .get("disabled_tools")
            .and_then(toml::Value::as_array)
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("read_file")))
        {
            self.never = true;
        }
        Ok(())
    }
    fn table(&mut self, tools: &toml::Table) -> Result<(), String> {
        let Some(settings) = tools.get("read_file").and_then(toml::Value::as_table) else {
            return Ok(());
        };
        self.never |= settings.get("permission").and_then(toml::Value::as_str) == Some("never");
        for name in ["denylist", "sensitive_patterns"] {
            for value in settings
                .get(name)
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
            {
                let pattern = value.as_str().ok_or("Cannot check file read rules.")?;
                self.pattern(pattern)?;
                let prefix = pattern.split(['*', '?', '[']).next().unwrap_or_default();
                if Path::new(prefix).is_absolute() {
                    let (base, tail) =
                        if prefix.ends_with(['/', '\\']) || prefix.len() == pattern.len() {
                            (prefix, &pattern[prefix.len()..])
                        } else {
                            let at = prefix.rfind(['/', '\\']).unwrap_or(0) + 1;
                            (&prefix[..at], &pattern[at..])
                        };
                    let real =
                        resolve(Path::new(base)).map_err(|_| "Cannot check file read rules.")?;
                    self.pattern(&format!(
                        "{}{}{}",
                        vw_platform::plain_path(&real).display(),
                        if tail.is_empty() { "" } else { "/" },
                        tail
                    ))?;
                }
            }
        }
        Ok(())
    }
    fn pattern(&mut self, text: &str) -> Result<(), String> {
        if self.patterns.len() >= 4096 || text.len() > 4096 {
            return Err("Cannot check file read rules.".into());
        }
        self.patterns.push(
            GlobBuilder::new(&text.replace('\\', "/"))
                .literal_separator(false)
                .backslash_escape(false)
                .case_insensitive(cfg!(windows))
                .build()
                .map_err(|_| "Cannot check file read rules.")?
                .compile_matcher(),
        );
        Ok(())
    }
    pub(crate) fn allows(&self, path: &Path) -> bool {
        let path = vw_platform::plain_path(path)
            .to_string_lossy()
            .replace('\\', "/");
        !self.never && !self.patterns.iter().any(|g| g.is_match(&path))
    }
}
fn read(path: &Path) -> Result<Option<String>, String> {
    let parent = path.parent().ok_or("Cannot check file read rules.")?;
    let name = path
        .file_name()
        .ok_or("Cannot check file read rules.")?
        .to_string_lossy();
    let file = match crate::search_walk::open(parent, &name) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot check file read rules.".into()),
    };
    let mut text = String::new();
    file.take((1 << 20) + 1)
        .read_to_string(&mut text)
        .map_err(|_| "Cannot check file read rules.")?;
    if text.len() > 1 << 20 {
        return Err("Cannot check file read rules.".into());
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fnmatch_denials_cover_children_sensitive_patterns_and_bad_config_fails_closed() {
        let mut rules = ReadRules::default();
        rules
            .parse("[tools.read_file]\ndenylist=['*/private/*']\nsensitive_patterns=['*.pem']")
            .unwrap();
        let root = Path::new(if cfg!(windows) {
            r"C:\project"
        } else {
            "/project"
        });
        assert!(!rules.allows(&root.join("private/nested/key.txt")));
        assert!(!rules.allows(&root.join("public.pem")));
        assert!(rules.allows(&root.join("public.txt")));
        assert!(rules.parse("invalid = [").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn an_absolute_denial_also_covers_its_resolved_alias() {
        let base = std::env::temp_dir().join(format!("vw-rule-alias-{}", std::process::id()));
        std::fs::create_dir_all(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("alias")).unwrap();
        let mut rules = ReadRules::default();
        rules
            .parse(&format!(
                "[tools.read_file]\ndenylist=['{}/*']",
                base.join("alias").display()
            ))
            .unwrap();
        // Search checks the resolved path, as search_tool does (macOS: the temp folder is under the /var link).
        assert!(!rules.allows(&resolve(&base.join("real")).unwrap().join("key.txt")));
        std::fs::remove_dir_all(base).unwrap();
    }
}
