//! Exact vault prompt text for Linux, macOS and Windows paths; uses the broker's shared formatter.
use super::summary;
use std::path::Path;

#[test]
fn typical_vault_prompt_on_each_platform() {
    for project in ["/home/ada/shop", "/Users/ada/shop", r"C:\Users\Ada\shop"] {
        let text = summary(
            "Deploy",
            project,
            Path::new(project),
            &["Deploy token".into()],
            "python",
            &["deploy.py".into(), "--site=demo".into()],
        );
        let quoted = vw_gate::dialog_quote(project);
        assert_eq!(
            text,
            format!(
                "Use secrets \"Deploy token\"?\n\nChat: \"Deploy\"\nFolder: {quoted}\nProgram: \"python\"\nArguments: \"deploy.py\", \"--site=demo\""
            )
        );
        assert!(text.chars().count() < 1200);
    }
}

#[test]
fn a_copy_folder_also_names_the_project() {
    let text = summary(
        "Deploy",
        "/home/ada/shop",
        Path::new("/home/ada/.vibe-work/copies/shop-1"),
        &["Deploy token".into()],
        "python",
        &[],
    );
    assert_eq!(
        text,
        "Use secrets \"Deploy token\"?\n\nChat: \"Deploy\"\nProject: \"/home/ada/shop\"\nFolder: \"/home/ada/.vibe-work/copies/shop-1\"\nProgram: \"python\"\nArguments: (none)"
    );
}
