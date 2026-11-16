//! The line-ending rules against git's `convert.c` (attribute decisions and both conversions).

use super::{Action, Config, action, parse_autocrlf, round_trips, to_git, to_worktree};

fn values(text: &str, eol: &str) -> [String; 6] {
    [
        text.into(),
        "unspecified".into(),
        eol.into(),
        "unspecified".into(),
        "unspecified".into(),
        "unspecified".into(),
    ]
}

const LF_ONLY: Config = Config {
    autocrlf: None,
    eol_crlf: Some(false),
};

#[test]
fn actions_follow_gits_attribute_rules() {
    let c = LF_ONLY;
    let text = |crlf| Action::Text { crlf };
    let guess = |crlf| Action::Guess { crlf };
    assert_eq!(action(&values("unspecified", "crlf"), c), text(true));
    assert_eq!(action(&values("set", "lf"), c), text(false));
    assert_eq!(action(&values("set", "unspecified"), c), text(false));
    assert_eq!(action(&values("input", "unspecified"), c), text(false));
    assert_eq!(action(&values("unset", "crlf"), c), Action::AsIs);
    assert_eq!(action(&values("auto", "crlf"), c), guess(true));
    assert_eq!(action(&values("auto", "unspecified"), c), guess(false));
    assert_eq!(
        action(&values("unspecified", "unspecified"), c),
        Action::AsIs
    );
    let autocrlf = Config {
        autocrlf: Some(true),
        eol_crlf: None,
    };
    assert_eq!(
        action(&values("unspecified", "unspecified"), autocrlf),
        guess(true)
    );
    assert_eq!(action(&values("set", "unspecified"), autocrlf), text(true));
    let input = Config {
        autocrlf: Some(false),
        eol_crlf: Some(true),
    };
    assert_eq!(
        action(&values("unspecified", "unspecified"), input),
        guess(false)
    );
    let eol_crlf = Config {
        autocrlf: None,
        eol_crlf: Some(true),
    };
    assert_eq!(action(&values("set", "unspecified"), eol_crlf), text(true));
    // core.eol alone does not make an unattributed file text.
    assert_eq!(
        action(&values("unspecified", "unspecified"), eol_crlf),
        Action::AsIs
    );

    let mut filtered = values("set", "crlf");
    filtered[3] = "lfs".into();
    assert_eq!(action(&filtered, c), Action::Unknown);
    let mut no_driver = values("set", "crlf");
    no_driver[3] = "set".into();
    assert_eq!(action(&no_driver, c), text(true));
    let mut ident = values("set", "crlf");
    ident[4] = "set".into();
    assert_eq!(action(&ident, c), Action::Unknown);
    let mut encoded = values("set", "crlf");
    encoded[5] = "UTF-16".into();
    assert_eq!(action(&encoded, c), Action::Unknown);
    let mut legacy = values("unspecified", "unspecified");
    legacy[1] = "set".into();
    assert_eq!(action(&legacy, c), text(false));
}

#[test]
fn autocrlf_values_read_like_git() {
    assert_eq!(parse_autocrlf("true"), Some(true));
    assert_eq!(parse_autocrlf("1"), Some(true));
    assert_eq!(parse_autocrlf("input"), Some(false));
    assert_eq!(parse_autocrlf("false"), None);
    assert_eq!(parse_autocrlf("0"), None);
    assert_eq!(parse_autocrlf("nonsense"), None);
}

#[test]
fn conversions_match_git() {
    let text = Action::Text { crlf: true };
    let guess = Action::Guess { crlf: true };
    assert_eq!(to_git(b"a\r\nb\r\n", text).unwrap(), b"a\nb\n");
    // Text keeps a lone CR; a guess calls the file binary for it.
    assert_eq!(to_git(b"a\rb\r\n", text).unwrap(), b"a\rb\n");
    assert_eq!(to_git(b"a\rb\r\n", guess), None);
    assert_eq!(to_git(b"a\0\r\n", guess), None);
    assert_eq!(to_git(b"a\r\nb\r\n", guess).unwrap(), b"a\nb\n");
    assert_eq!(to_git(b"plain\n", text), None);
    assert_eq!(to_git(b"", text), None);
    assert_eq!(to_git(b"a\r\n", Action::AsIs), None);

    assert_eq!(to_worktree(b"a\nb\n", text), b"a\r\nb\r\n");
    assert_eq!(
        to_worktree(b"a\nb\n", Action::Text { crlf: false }),
        b"a\nb\n"
    );
    // A guess leaves a blob holding a CR alone.
    assert_eq!(to_worktree(b"a\r\nb\n", guess), b"a\r\nb\n");
    assert_eq!(to_worktree(b"a\r\nb\n", text), b"a\r\nb\r\n");
}

#[test]
fn only_a_byte_exact_round_trip_is_unchanged() {
    let crlf = Action::Text { crlf: true };
    let lf = Action::Text { crlf: false };
    let base = |want: &'static [u8]| move |stored: &[u8]| stored == want;
    assert!(round_trips(b"one\r\ntwo\r\n", crlf, base(b"one\ntwo\n")));
    // Mixed line endings in an eol=crlf file: a checkout would write CRLF everywhere.
    assert!(!round_trips(b"one\r\ntwo\n", crlf, base(b"one\ntwo\n")));
    // An eol=lf file saved with CRLF: a checkout would write LF.
    assert!(!round_trips(b"one\r\ntwo\r\n", lf, base(b"one\ntwo\n")));
    // A real edit.
    assert!(!round_trips(b"one\r\nthree\r\n", crlf, base(b"one\ntwo\n")));
    // CR CR LF: stored as CR LF, which a checkout leaves alone.
    assert!(!round_trips(b"a\r\r\n", crlf, base(b"a\r\n")));
}
