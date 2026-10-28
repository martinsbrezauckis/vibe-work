//! Short vault confirmation text, with names and the requested command, never file contents or values.
//! Uses the common dialog quoting; called by the secret broker and checked on every OS's text path.
use std::path::Path;

pub(super) fn summary(
    title: &str,
    project: &str,
    cwd: &Path,
    names: &[String],
    program: &str,
    args: &[String],
) -> String {
    let quote = vw_gate::dialog_quote;
    // A secret bound to several variables is named once.
    let mut seen = std::collections::BTreeSet::new();
    let names = names
        .iter()
        .filter(|s| seen.insert(s.as_str()))
        .map(|s| quote(s))
        .collect::<Vec<_>>()
        .join(", ");
    let args = args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ");
    let cwd = cwd.to_string_lossy();
    let folder = crate::secret_path::display(&cwd);
    let project = crate::secret_path::display(project);
    // The project line names the project an Always answer applies to; it is shown only when it is not
    // the folder itself (a copy or a subfolder).
    let project = if project == folder {
        String::new()
    } else {
        format!("Project: {}\n", quote(&project))
    };
    format!(
        "Use secrets {names}?\n\nChat: {}\n{project}Folder: {}\nProgram: {}\nArguments: {}",
        vw_gate::dialog_chat_quote(title),
        quote(&folder),
        quote(program),
        if args.is_empty() { "(none)" } else { &args },
    )
}

#[cfg(test)]
#[path = "secret_dialog_tests.rs"]
mod tests;
