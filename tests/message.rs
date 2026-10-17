use tamper::cases::Compare;
use tamper::message::{matches, normalise};

#[test]
fn whitespace_runs_collapse() {
    assert_eq!(normalise("a   b\n\n  c\t d"), "a b c d");
    assert_eq!(
        normalise("  leading and trailing  "),
        "leading and trailing"
    );
}

#[test]
fn a_wrapped_assertion_message_still_matches() {
    // This is the whole reason for normalising: nix wraps its output, and a
    // healthy check looked dead because the pattern spanned the break.
    let out = "error: Failed assertions:\n- Gast media-01 ohne numerische\n  UID-Basis\n";
    assert!(matches(out, "ohne numerische UID-Basis", Compare::Regex));
}

#[test]
fn a_dot_stands_in_for_a_backtick() {
    // Measured: 30 of 522 patterns carry metacharacters, and several use `.`
    // where the message has a backtick. A substring compare would break these.
    let out = "error: die Unit setzt `set -e` nicht";
    assert!(matches(out, "setzt .set -e.", Compare::Regex));
    assert!(!matches(out, "setzt .set -e.", Compare::Literal));
}

#[test]
fn escaped_brackets_are_literal_in_a_regex() {
    let out = "error: der Wert steht im [DEFAULT]-Block";
    assert!(matches(out, r"steht im \[DEFAULT\]-Block", Compare::Regex));
}

#[test]
fn regex_ignores_case_and_literal_does_not() {
    // grep -qi versus grep -qF: the two shell functions differ, so the cases
    // have to carry which one they mean.
    let out = "error: Pfad-Invariante verletzt";
    assert!(matches(out, "pfad-invariante", Compare::Regex));
    assert!(!matches(out, "pfad-invariante", Compare::Literal));
    assert!(matches(out, "Pfad-Invariante", Compare::Literal));
}

#[test]
fn literal_also_survives_a_line_break() {
    let out = "PROBE-NEIN: fw4 check\n  meldet eine\n  ungueltige Zone";
    assert!(matches(
        out,
        "meldet eine ungueltige Zone",
        Compare::Literal
    ));
}

#[test]
fn braces_are_literal_to_grep_but_not_to_this_engine() {
    // Measured 2026-09-20: `fehlt {EntryData}` — one pattern out of 522 —
    // is a plain string to grep's BRE, and this engine refuses it with
    // "repetition quantifier expects a valid decimal". So it does not merely
    // behave differently, it does not compile at all. `cases::validate`
    // catches that at load time; the importer escapes the braces, which
    // keeps the case-insensitivity that `grep -qi` had.
    let out = "error: die Vorlage fehlt {EntryData} und bricht ab";
    assert!(!matches(out, "fehlt {EntryData}", Compare::Regex));
    assert!(matches(out, r"fehlt \{EntryData\}", Compare::Regex));
    assert!(matches(out, "fehlt {EntryData}", Compare::Literal));
}

#[test]
fn a_pattern_that_is_not_there_does_not_match() {
    assert!(!matches(
        "error: something else",
        "ohne Snapshot-Abdeckung",
        Compare::Regex
    ));
}

// --- check ids: markers, not patterns ---------------------------------------

use tamper::cases::{Case, Lever};
use tamper::message::{expected, marker, missing_checks};

fn case_with(expect: &str, checks: &[&str]) -> Case {
    Case {
        id: "x".into(),
        name: "x".into(),
        target: "server".into(),
        expect: expect.into(),
        checks: checks.iter().map(|s| s.to_string()).collect(),
        compare: Compare::Regex,
        why: "x".into(),
        green: false,
        levers: vec![Lever::Sed {
            file: "a.nix".into(),
            sed: "s|a|b|".into(),
        }],
    }
}

const OUT: &str = "error: Failed assertions:\n- [P-103] Pruefung 103: Gast media-01 ohne\n       Snapshot-Abdeckung\n- [P-103b] Pruefung 103b: etwas anderes\n";

#[test]
fn the_marker_of_a_check_id_is_the_id_in_brackets() {
    assert_eq!(marker("P-103"), "[P-103]");
}

#[test]
fn a_present_marker_satisfies_a_checks_only_case() {
    assert!(expected(OUT, &case_with("", &["P-103"])));
}

#[test]
fn a_missing_marker_does_not() {
    assert!(!expected(OUT, &case_with("", &["P-104"])));
    assert_eq!(missing_checks(OUT, &["P-104".to_string()]), vec!["P-104"]);
}

#[test]
fn an_id_is_a_value_not_a_prefix() {
    // The whole reason for the field: `Pruefung 103` as a pattern also hits
    // 103b. `[P-103]` does not hit `[P-103b]`, and the other way round.
    let only_b = "error: Failed assertions:\n- [P-103b] Pruefung 103b: etwas anderes\n";
    assert!(matches(only_b, "Pruefung 103", Compare::Regex));
    assert!(!expected(only_b, &case_with("", &["P-103"])));
    assert!(expected(only_b, &case_with("", &["P-103b"])));
}

#[test]
fn every_named_check_must_have_fired() {
    assert!(expected(OUT, &case_with("", &["P-103", "P-103b"])));
    assert!(!expected(OUT, &case_with("", &["P-103", "P-999"])));
    assert_eq!(
        missing_checks(OUT, &["P-103".into(), "P-999".into(), "P-998".into()]),
        vec!["P-999", "P-998"]
    );
}

#[test]
fn markers_are_case_sensitive_and_literal() {
    assert!(!expected(OUT, &case_with("", &["p-103"])));
    // `expect` would read `[P-103]` as a character class; the marker is literal.
    let bracketless = "error: P-103 Pruefung 103: ohne Snapshot-Abdeckung";
    assert!(!expected(bracketless, &case_with("", &["P-103"])));
}

#[test]
fn a_marker_survives_the_wrapped_output() {
    // Normalised like `expect`: a marker carries no whitespace, so nix cannot
    // break it, and the text around it collapses to single spaces.
    let wrapped = "error:\n  - [P-103]\n       Pruefung 103\n";
    assert!(expected(wrapped, &case_with("", &["P-103"])));
}

#[test]
fn expect_and_checks_together_must_both_hold() {
    assert!(expected(
        OUT,
        &case_with("ohne Snapshot-Abdeckung", &["P-103"])
    ));
    // marker there, message not
    assert!(!expected(OUT, &case_with("etwas ganz anderes", &["P-103"])));
    // message there, marker not
    assert!(!expected(
        OUT,
        &case_with("ohne Snapshot-Abdeckung", &["P-999"])
    ));
}

#[test]
fn a_case_with_neither_never_passes() {
    // Refused by `cases::validate`; should one slip through, it must not be `ok`.
    assert!(!expected(OUT, &case_with("", &[])));
}

#[test]
fn without_checks_the_old_rule_stands() {
    assert!(expected(OUT, &case_with("ohne Snapshot-Abdeckung", &[])));
    assert!(!expected(OUT, &case_with("etwas ganz anderes", &[])));
}
