//! Conservative feedback redaction; the core calls this before preview, events and submission.
//! Uses known local usernames, environment credentials and the existing vault-value guard; never reads chat text.
use crate::{CoreError, ErrorCode, FeedbackDraft, core::Shared};

#[derive(serde::Deserialize)]
struct Rule {
    pattern: String,
    flags: String,
    replacement: String,
}
#[derive(serde::Deserialize)]
struct Specification {
    rules: Vec<Rule>,
}
static RULES: std::sync::LazyLock<Result<Vec<(regex::Regex, String)>, String>> =
    std::sync::LazyLock::new(|| {
        let specification: Specification =
            serde_json::from_str(include_str!("../feedback-redaction-rules.json"))
                .map_err(|error| error.to_string())?;
        specification
            .rules
            .into_iter()
            .map(|rule| {
                regex::RegexBuilder::new(&rule.pattern)
                    .case_insensitive(rule.flags.contains('i'))
                    .build()
                    .map(|pattern| (pattern, rule.replacement))
                    .map_err(|error| error.to_string())
            })
            .collect()
    });

pub(crate) fn text(input: &str, identities: &[String]) -> String {
    let mut out = input.to_owned();
    for name in identities.iter().filter(|s| !s.is_empty()) {
        if let Ok(re) = regex::RegexBuilder::new(&regex::escape(name))
            .case_insensitive(true)
            .build()
        {
            out = re.replace_all(&out, "[redacted]").into_owned();
        }
    }
    // Invalid embedded rules fail closed rather than silently omitting a redaction family.
    let Ok(rules) = &*RULES else {
        return "[redacted]".into();
    };
    for (pattern, replacement) in rules {
        out = pattern.replace_all(&out, replacement.as_str()).into_owned();
    }

    out
}
pub(crate) fn draft(
    shared: &Shared,
    mut draft: FeedbackDraft,
    required: bool,
) -> Result<FeedbackDraft, CoreError> {
    let identities: Vec<String> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.to_str()?;
            let value = value.to_str()?;
            let upper = key.to_ascii_uppercase();
            let identity =
                ["USER", "USERNAME", "LOGNAME", "HOME", "USERPROFILE"].contains(&upper.as_str());
            let secret = ["KEY", "TOKEN", "PASSWORD", "SECRET"]
                .iter()
                .any(|suffix| upper.ends_with(suffix))
                && value.len() >= 8;
            (identity || secret).then(|| value.to_owned())
        })
        .collect();
    for (value, cap, needed) in [
        (&mut draft.title, 160, required),
        (&mut draft.description, 8000, required),
        (&mut draft.remarks, 2000, false),
    ] {
        if value.len() > cap
            || (needed && value.trim().is_empty())
            || value
                .chars()
                .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
        {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "Check the report length and required fields.",
            ));
        }
        *value = text(
            &crate::title_guard::withhold_text(shared, value.trim()),
            &identities,
        );
        if value.len() > cap {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "The redacted report is too long. Shorten it and review again.",
            ));
        }
    }
    Ok(draft)
}
