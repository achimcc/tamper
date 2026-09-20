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
