//! Complete secret-run confirmation text from the pinned command and secret names.
//!
//! Depends on: vw-gate quoting and `secret_pin`. Called by: `secret_run` before native consent.
//! Every displayed argument describes the pinned argv that executes; values never enter this text.

use std::path::Path;

use super::secret_pin::Pinned;
use super::secret_run::ALWAYS_LABEL;

pub(super) struct Dialog<'a> {
    pub title: &'a str,
    pub project: &'a str,
    pub cwd: &'a Path,
    pub purpose: &'a str,
    pub timeout_s: u64,
    pub names: &'a [String],
    pub pinned: &'a Pinned,
}

/// A lexical abbreviation of a canonical path, keeping its spelling and separator intact.
/// No filesystem reads: only a complete project prefix with a path boundary can be replaced.
fn relative(project: &str, path: &str) -> Option<String> {
    let windows = project.starts_with(r"\\")
        || (project.as_bytes().get(1) == Some(&b':')
            && matches!(project.as_bytes().get(2), Some(b'/' | b'\\'))
            && project
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic));
    let separator = |ch| ch == '/' || (windows && ch == '\\');
    if project.is_empty() || (!windows && !project.starts_with('/')) {
        return None;
    }
    let suffix = path.strip_prefix(project)?;
    if suffix.is_empty() {
        return Some(".".to_owned());
    }
    // Keep the boundary in the abbreviation: replacing '.' with Project recovers the exact text.
    let boundary = if project.ends_with(separator) {
        ""
    } else if suffix.starts_with(separator) {
        &suffix[..1]
    } else {
        return None;
    };
    let rest = suffix.strip_prefix(boundary)?;
    if rest.split(separator).any(|part| matches!(part, "." | "..")) {
        return None;
    }
    Some(format!(".{suffix}"))
}

fn project_path(project: &str, path: &str) -> Option<String> {
    let short = |text: String| format!("project path {}", vw_gate::dialog_quote(&text));
    if let Some(relative) = relative(project, path) {
        return Some(short(relative));
    }
    // Session folders use everyday Windows spelling; pinned files retain Rust's extended spelling.
    // Keep that distinction visible: the displayed pieces recover the exact executed argument.
    if let Some(plain) = path.strip_prefix(r"\\?\")
        && let Some(relative) = relative(project, plain)
    {
        return Some(format!(
            "{} + {}",
            vw_gate::dialog_quote(r"\\?\"),
            short(relative)
        ));
    }
    if project.starts_with(r"\\")
        && let Some(unc) = path.strip_prefix(r"\\?\UNC\")
        && let Some(relative) = relative(project, &format!(r"\\{unc}"))
    {
        return Some(format!("extended UNC {}", short(relative)));
    }
    None
}

fn path_text(project: &str, path: &str) -> String {
    project_path(project, path).unwrap_or_else(|| vw_gate::dialog_quote(path))
}

/// Only pinned file arguments may be abbreviated. A string, URL or option containing the project text
/// remains literal; the ordered list also keeps empty arguments and '--option=' concatenation explicit.
fn argument_text(project: &str, arg: &str, pinned: &Pinned) -> String {
    let (prefix, candidate) = arg
        .split_once('=')
        .filter(|_| arg.starts_with('-'))
        .map_or((None, arg), |(option, value)| (Some(option), value));
    let file = pinned
        .files
        .iter()
        .any(|(path, _)| path.to_string_lossy() == candidate);
    if file && let Some(path) = project_path(project, candidate) {
        return prefix.map_or_else(
            || path.clone(),
            |option| format!("{} + {path}", vw_gate::dialog_quote(&format!("{option}="))),
        );
    }
    vw_gate::dialog_quote(arg)
}

pub(super) fn summary(dialog: &Dialog<'_>) -> String {
    let Dialog {
        title,
        project,
        cwd,
        purpose,
        timeout_s,
        names,
        pinned,
    } = dialog;
    let args = pinned
        .args
        .iter()
        .map(|arg| argument_text(project, arg, pinned))
        .collect::<Vec<_>>()
        .join(", ");
    let files: Vec<_> = pinned
        .files
        .iter()
        .map(|(path, _)| path_text(project, &path.to_string_lossy()))
        .collect();
    let unc = if files.iter().any(|text| text.starts_with("extended UNC ")) {
        "\nExtended UNC paths replace Project's leading '\\\\' with '\\\\?\\UNC\\'."
    } else {
        ""
    };
    format!(
        "The agent of session {} wants to give your secret {} to this program. It runs directly, without a shell, \
         and can do anything with the values. The agent never sees them; output containing them is withheld. \
         \"{}\" lets this project's agents repeat this exact program, arguments, folder and files without asking. \
         Other commands ask; revoke in Settings > Project secrets.\n\n\
         Project: {}\nA leading '.' in a project path means the Project folder; '+' joins parts. The rest is exact.{unc}\n\
         Folder: {}\nPurpose given by agent: {}\nStop after: {} seconds\nFiles checked: {}\n\nProgram: {}\nArguments (exact): {}",
        vw_gate::dialog_quote(title),
        names.join(", "),
        ALWAYS_LABEL,
        vw_gate::dialog_quote(project),
        path_text(project, &cwd.to_string_lossy()),
        vw_gate::dialog_quote(purpose),
        timeout_s,
        files.join(", "),
        path_text(project, &pinned.program.to_string_lossy()),
        if args.is_empty() { "(none)" } else { &args },
    )
}

#[cfg(test)]
mod tests {
    use super::{Dialog, argument_text, path_text, relative, summary};
    use crate::agent_tools::secret_pin::Pinned;
    use std::path::PathBuf;

    const REQUESTER_LINE: &str =
        "The agent of a session, not a click in this window, asks for this.\n\n";

    fn deep_folder_dialog(windows: bool) {
        let prefix = if windows { "C:\\work\\" } else { "/work/" };
        let project = format!("{prefix}{}", "p".repeat(200 - prefix.len()));
        let separator = if windows { '\\' } else { '/' };
        let script = format!("{project}{separator}printer.sh");
        let program = if windows {
            r"C:\Windows\System32\sh.exe"
        } else {
            "/usr/bin/dash"
        };
        let pinned = Pinned {
            program: PathBuf::from(program),
            args: vec![script.clone()],
            links: vec![(PathBuf::from(&script), PathBuf::from(&script))],
            files: vec![
                (PathBuf::from(program), String::new()),
                (PathBuf::from(&script), String::new()),
            ],
            fingerprint: String::new(),
        };
        let names = vec!["\"Deploy token\" into \"DEPLOY_TOKEN\"".to_owned()];
        let text = summary(&Dialog {
            title: "Deploy",
            project: &project,
            cwd: &PathBuf::from(&project),
            purpose: "deploy the test site",
            timeout_s: 120,
            names: &names,
            pinned: &pinned,
        });
        println!("{} characters:\n{text}", text.chars().count());
        assert!(
            text.chars().count() + REQUESTER_LINE.len() < super::super::secret_run::DIALOG_MAX,
            "dialog over DIALOG_PARAMS_MAX_CHARS"
        );
        for fact in [
            "Deploy",
            "Deploy token",
            "DEPLOY_TOKEN",
            "printer.sh",
            "Program:",
            "Arguments (exact):",
            "Folder:",
            "Files checked:",
            "deploy the test site",
            "120 seconds",
            "Always for this command",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        assert_eq!(text.matches(&vw_gate::dialog_quote(&project)).count(), 1);
        assert!(text.contains(&vw_gate::dialog_quote(program)));
    }

    #[test]
    fn deep_unix_folder_fits_with_every_fact() {
        deep_folder_dialog(false);
    }

    #[test]
    fn deep_windows_folder_fits_with_every_fact() {
        deep_folder_dialog(true);
    }

    #[test]
    fn windows_pinned_paths_fit_with_a_plain_project_folder() {
        for prefix in [r"C:\work\", r"\\server\share\"] {
            let project = format!("{prefix}{}", "p".repeat(200 - prefix.len()));
            let extended = if prefix.starts_with(r"\\") {
                format!(r"\\?\UNC\{}", &project[2..])
            } else {
                format!(r"\\?\{project}")
            };
            let program = format!(r"{extended}\run.exe");
            let script = format!(r"{extended}\deploy.ps1");
            let pinned = Pinned {
                program: PathBuf::from(&program),
                args: vec![script.clone()],
                links: Vec::new(),
                files: vec![
                    (PathBuf::from(&program), String::new()),
                    (PathBuf::from(&script), String::new()),
                ],
                fingerprint: String::new(),
            };
            let text = summary(&Dialog {
                title: "Deploy",
                project: &project,
                cwd: &PathBuf::from(&project),
                purpose: "deploy the test site",
                timeout_s: 120,
                names: &["\"Deploy token\" into \"DEPLOY_TOKEN\"".to_owned()],
                pinned: &pinned,
            });
            println!(
                "Windows mixed spelling: {} characters\n{text}",
                text.chars().count()
            );
            assert!(
                text.chars().count() + REQUESTER_LINE.len() < super::super::secret_run::DIALOG_MAX
            );
            assert!(text.contains("run.exe") && text.contains("deploy.ps1"));
            assert!(text.contains("Arguments (exact):"));
            assert_eq!(text.matches(&vw_gate::dialog_quote(&project)).count(), 1);
            let program_text = if prefix.starts_with(r"\\") {
                assert!(text.contains(r"replace Project's leading '\\' with '\\?\UNC\'"));
                r#"extended UNC project path ".\run.exe""#.replace('\\', "\\\\")
            } else {
                format!(
                    "{} + project path {}",
                    vw_gate::dialog_quote(r"\\?\"),
                    vw_gate::dialog_quote(r".\run.exe")
                )
            };
            assert!(text.contains(&format!("Program: {program_text}")), "{text}");
            assert!(
                text.contains(&format!("Files checked: {program_text},")),
                "{text}"
            );
            let script_text = program_text.replace("run.exe", "deploy.ps1");
            assert!(
                text.contains(&format!("Arguments (exact): {script_text}")),
                "{text}"
            );
            assert_eq!(pinned.args, [script]);
        }
    }

    #[test]
    fn project_path_replacement_recovers_the_exact_spelling() {
        for (project, path, expected) in [
            ("/work/project", "/work/project", "."),
            (
                "/work/project",
                "/work/project/jobs/deploy.sh",
                "./jobs/deploy.sh",
            ),
            (
                r"C:\work\project",
                r"C:\work\project\jobs\deploy.ps1",
                r".\jobs\deploy.ps1",
            ),
            (
                r"\\server\share\project",
                r"\\server\share\project\deploy.ps1",
                r".\deploy.ps1",
            ),
            (
                r"\\?\C:\project",
                r"\\?\C:\project\deploy.ps1",
                r".\deploy.ps1",
            ),
            ("/work/a\\b", "/work/a\\b/deploy.sh", "./deploy.sh"),
            ("/", "/deploy.sh", ".deploy.sh"),
        ] {
            let short = relative(project, path).unwrap();
            assert_eq!(short, expected);
            assert_eq!(short.replacen('.', project, 1), path);
        }
        for (project, outside) in [
            ("/work/project", "/work/project-other/deploy.sh"),
            ("/work/project", "/work/project/../elsewhere/deploy.sh"),
            ("/work/project", "/work/project\\deploy.sh"),
            (r"C:\work\project", r"C:\work\project-other\deploy.ps1"),
            (
                r"C:\work\project",
                r"C:\work\project\..\elsewhere\deploy.ps1",
            ),
            (r"C:\work\project", r"D:\work\project\deploy.ps1"),
            ("C:project", "C:project/file"),
            ("project", "project/file"),
            ("", "/deploy.sh"),
        ] {
            assert_eq!(relative(project, outside), None);
            assert_eq!(path_text(project, outside), vw_gate::dialog_quote(outside));
        }
    }

    #[test]
    fn arguments_keep_order_empty_values_options_and_literal_path_text() {
        let project = "/work/project";
        let pinned = Pinned {
            program: PathBuf::from("/work/project/tools/run"),
            args: vec![
                "/work/project/scripts/deploy.sh".to_owned(),
                "--config=/work/project/config.json".to_owned(),
                String::new(),
                "a  \"b\n".to_owned(),
                "/work/project/not-a-file".to_owned(),
                "--label=/work/project/not-a-file".to_owned(),
                "/work/project-other/deploy.sh".to_owned(),
            ],
            links: Vec::new(),
            files: vec![
                (PathBuf::from("/work/project/tools/run"), String::new()),
                (
                    PathBuf::from("/work/project/scripts/deploy.sh"),
                    String::new(),
                ),
                (PathBuf::from("/work/project/config.json"), String::new()),
                (
                    PathBuf::from("/work/project-other/deploy.sh"),
                    String::new(),
                ),
            ],
            fingerprint: String::new(),
        };
        let args: Vec<_> = pinned
            .args
            .iter()
            .map(|arg| argument_text(project, arg, &pinned))
            .collect();
        assert_eq!(
            args,
            [
                "project path \"./scripts/deploy.sh\"",
                "\"--config=\" + project path \"./config.json\"",
                "\"\"",
                "\"a  \\\"b\\u000A\"",
                "\"/work/project/not-a-file\"",
                "\"--label=/work/project/not-a-file\"",
                "\"/work/project-other/deploy.sh\"",
            ]
        );
        let text = summary(&Dialog {
            title: "Deploy",
            project,
            cwd: &PathBuf::from("/work/project/jobs"),
            purpose: "run deploy",
            timeout_s: 42,
            names: &[
                "\"First\" into \"A\"".to_owned(),
                "\"Second\" into \"B\"".to_owned(),
            ],
            pinned: &pinned,
        });
        assert!(text.contains("Program: project path \"./tools/run\""));
        assert!(text.contains("Folder: project path \"./jobs\""));
        assert!(text.contains(&format!("Arguments (exact): {}", args.join(", "))));
        assert!(text.contains("Files checked: project path \"./tools/run\", project path \"./scripts/deploy.sh\", project path \"./config.json\", \"/work/project-other/deploy.sh\""));
        assert!(text.contains("\"First\" into \"A\", \"Second\" into \"B\""));
    }
}
