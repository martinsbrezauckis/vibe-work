//! Windows user builds: at startup, an installed copy that an interrupted install or update left incomplete (a program
//! it ships beside it missing or cut short, or the Windows uninstall entry absent, `vw_host::install_check`) tells the person once, in the app's own native
//! dialog, to run the installer again. The app keeps running; nothing is repaired or deleted here. A shipped program
//! that is present but cannot start (Windows cannot load it) is reported the same way, naming what stops working; when
//! it is the agent tools program the launch also refuses every tool call with the reason ([`tools_problem`]).
//! Depends on: `vw_host::install_check`, the native dialogs. Called by: lib (setup).

use tauri::{AppHandle, Manager};

/// The notice's words; used only by this dialog.
const HEADING: &str = "Vibe Work is not installed completely.";
const OK: &str = "OK";

/// The notice text for the missing or damaged program `name` (heading, then the detail).
pub(crate) fn text(name: &str) -> String {
    format!(
        "{HEADING}\n\n{name} is missing or damaged, so some features will not work. Run the Vibe Work installer \
         again to repair the installation."
    )
}

/// The notice text for the program `name` that is installed but cannot start (`problem`: why, from
/// `vw_host::install_check::start_problem`): what stops working, then the repair.
pub(crate) fn start_text(name: &str, problem: &str) -> String {
    use vw_vocabulary::texts::{CHROME_NAME, EDGE_NAME};
    let lost = match name {
        "vw-agent-tools.exe" => "the agent cannot use any tool".to_owned(),
        "vw-video-helper.exe" => "the agent cannot look at videos".to_owned(),
        _ => format!("{CHROME_NAME} and {EDGE_NAME} cannot connect"),
    };
    format!(
        "{HEADING}\n\n{name} cannot start on this computer ({problem}), so {lost}. Run the Vibe Work installer again \
         to repair the installation."
    )
}

/// This run's start probes of the shipped programs that cannot start (file name, why), made once on first use
/// (Windows installed copies of user builds; empty elsewhere): the notice, the tools refusal and the browser helper's
/// refusal all read the same result.
fn probes(app: &AppHandle) -> &'static [(&'static str, String)] {
    static PROBES: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();
    PROBES.get_or_init(|| {
        if !cfg!(windows) || crate::startup::DEV_BUILD {
            return Vec::new();
        }
        let Ok(exe) = std::env::current_exe() else {
            return Vec::new();
        };
        let problems = vw_host::install_check::start_problems(&exe, default_folder(app).as_deref());
        for (program, problem) in &problems {
            tracing::warn!(program, %problem, "a shipped program cannot start");
        }
        problems
    })
}

/// Why this run's `vw-agent-tools` cannot start (Windows installed copies of user builds): it is missing, or Windows
/// cannot load it. The launch then refuses every tool call with this reason (`ProductInputs::tools_problem`).
pub(crate) fn tools_problem(app: &AppHandle) -> Option<String> {
    if !cfg!(windows) || crate::startup::DEV_BUILD {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if !vw_host::install_check::installed_copy(&exe, default_folder(app).as_deref()) {
        return None;
    }
    if !exe.parent()?.join(vw_host::agent_tools::BINARY).is_file() {
        tracing::warn!("the agent tools cannot start: the program is missing");
        return Some("it is missing".to_owned());
    }
    probe(app, vw_host::agent_tools::BINARY)
}

/// Why this run's browser helper (`chrome-control.exe`) cannot start; its agent refusal says so
/// (`ChromeControlConfig::helper_start_problem`).
pub(crate) fn chrome_problem(app: &AppHandle) -> Option<String> {
    probe(app, vw_host::install_check::CHROME_HELPER[2])
}

fn probe(app: &AppHandle, name: &str) -> Option<String> {
    probes(app)
        .iter()
        .find(|(program, _)| *program == name)
        .map(|(_, problem)| problem.clone())
}

/// The installer's default folder: the per-user programs folder named after the product.
fn default_folder(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .local_data_dir()
        .ok()
        .zip(app.config().product_name.clone())
        .map(|(data, name)| data.join(name))
}

/// The notice text when the installer's Windows uninstall entry is missing (heading, then the detail).
pub(crate) fn entry_text() -> String {
    format!(
        "{HEADING}\n\nIts Windows uninstall entry is missing, so it is not listed in Settings > Apps and cannot be removed \
         there. Run the Vibe Work installer again to repair the installation."
    )
}

/// Checks the running copy and, when it is incomplete, shows the notice (without waiting for it).
pub(crate) fn check(app: &AppHandle) {
    if !cfg!(windows) || crate::startup::DEV_BUILD {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let default_folder = default_folder(app);
    let notice = if let Some(name) =
        vw_host::install_check::damaged_helper(&exe, default_folder.as_deref())
    {
        tracing::warn!(program = name, "the installation is incomplete");
        text(name)
    } else if let Some((name, problem)) = probes(app).first() {
        start_text(name, problem)
    } else if missing_uninstall_entry(app, &exe, default_folder.as_deref()) {
        tracing::warn!("the installation is incomplete: no Windows uninstall entry");
        entry_text()
    } else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::dialogs::ask_person(&app, notice, &[OK.to_owned()]).await;
    });
}

/// An installed copy whose installer-written uninstall entry is absent from the registry (Windows only).
fn missing_uninstall_entry(
    app: &AppHandle,
    exe: &std::path::Path,
    default_folder: Option<&std::path::Path>,
) -> bool {
    #[cfg(windows)]
    {
        let Some(product) = app.config().product_name.clone() else {
            return false;
        };
        vw_host::install_check::installed_copy(exe, default_folder)
            && !vw_host::install_check::uninstall_entry_present(&product)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, exe, default_folder);
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_notice_names_the_program_and_the_repair() {
        let text = super::text("vw-video-helper.exe");
        assert!(text.starts_with("Vibe Work is not installed completely.\n\n"));
        assert!(text.contains("vw-video-helper.exe is missing or damaged"));
        assert!(text.contains("Run the Vibe Work installer again"));
    }

    #[test]
    fn the_start_notice_names_the_program_what_stops_and_the_repair() {
        let problem = "Windows could not load it: error 0xC0000135, a DLL it needs is missing";
        let text = super::start_text("vw-agent-tools.exe", problem);
        assert_eq!(
            text,
            "Vibe Work is not installed completely.\n\nvw-agent-tools.exe cannot start on this computer (Windows could \
             not load it: error 0xC0000135, a DLL it needs is missing), so the agent cannot use any tool. Run the Vibe \
             Work installer again to repair the installation."
        );
        assert!(
            super::start_text("chrome-control.exe", problem).contains(&format!(
                "so {} and {} cannot connect.",
                vw_vocabulary::texts::CHROME_NAME,
                vw_vocabulary::texts::EDGE_NAME
            ))
        );
        assert!(
            super::start_text("vw-video-helper.exe", problem)
                .contains("so the agent cannot look at videos.")
        );
    }

    #[test]
    fn the_entry_notice_says_what_is_missing_and_the_repair() {
        let text = super::entry_text();
        assert!(text.starts_with("Vibe Work is not installed completely.\n\n"));
        assert!(text.contains("Windows uninstall entry is missing"));
        assert!(text.contains("Settings > Apps"));
        assert!(text.contains("Run the Vibe Work installer again"));
    }
}
