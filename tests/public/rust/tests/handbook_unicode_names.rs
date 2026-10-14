//! Untrusted Unicode handbook names are rejected without panicking.

/// A file name is untrusted text. The dated and numbered name checks cut the name at fixed byte positions; a name
/// with Latvian, Cyrillic or emoji text puts one of those cuts inside a character. The answer must be a plain
/// "no match", never a panic, and ordinary ASCII names must still match.
#[test]
fn names_with_non_ascii_text_do_not_match_and_do_not_panic() {
    use vw_pages::handbook::format::{kind, name_matches};
    let report = kind("report").unwrap();
    let decision = kind("decision").unwrap();
    let report_names = [
        // Byte 10 is the second half of the `ā`: a malformed dated name with a cut inside a character.
        "reports/123456789ā-test.html",
        "reports/2026-10-0ā-test.html",
        "reports/2026-10-0🙂-test.html",
        "reports/xабвгдеж-тест.html",
        // The date part is fine, the character right after it is not a hyphen.
        "reports/2026-10-09š-atskaite.html",
        "reports/2026-10-09-pārskats.html",
        "reports/2026-10-09-отчёт.html",
        // Plain malformed ASCII names stay refused.
        "reports/2026-10-09.html",
        "reports/2026-13-09-x.html",
    ];
    for path in report_names {
        assert!(!name_matches(report, path), "{path}");
    }
    let decision_names = [
        "decisions/0001-ā.html",
        "decisions/0001-lēmums.html",
        "decisions/ā001-x.html",
        "decisions/000ā-x.html",
        "decisions/0001ā-x.html",
        "decisions/🙂🙂-x.html",
    ];
    for path in decision_names {
        assert!(!name_matches(decision, path), "{path}");
    }
    assert!(name_matches(report, "reports/2026-10-09-parser-bench.html"));
    assert!(name_matches(
        decision,
        "decisions/0001-parse-with-regex.html"
    ));
}
