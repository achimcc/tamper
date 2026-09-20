//! The rules against REAL output, not invented input.
//!
//! Every other test in this repository hands the rules a string that was
//! written to make them pass. That tests the rule, not the world — the same
//! gap as a promtool test that asserts a series somebody typed out by hand.
//! The files under `tests/antworten/` were cut from the acceptance run of
//! 2026-09-20: genuine `nix build` output from a sabotaged tree, a genuine
//! `nix-instantiate --parse` failure, a genuine green build. Only the two
//! lotse framing lines and the throwaway tree's path were removed.
//!
//! What they are worth is the detail nobody invents: nix indents every
//! continuation line of an assertion message with seven spaces, breaks the
//! prose where it likes, and puts a twenty-line stack trace in front of the
//! message the case is looking for.

use std::path::Path;

use tamper::build::{classify, Outcome};
use tamper::cases::Compare;
use tamper::message::matches;
use tamper::parse::from_stderr;

fn antwort(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/antworten")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The case is `treff-ohne-email-scope`; its pattern writes `.` where the
/// message has a backtick, which is exactly why the comparison is a regular
/// expression and not a substring.
#[test]
fn a_real_assertion_matches_its_pattern_with_a_dot_for_a_backtick() {
    let out = antwort("assertion-scope-mit-backtick.txt");
    assert!(out.contains("gibt den `email`-Scope nicht frei"));
    assert!(matches(&out, "gibt den .email.-Scope nicht frei", Compare::Regex));
    // And the substring compare would have called this healthy check dead.
    assert!(!matches(
        &out,
        "gibt den .email.-Scope nicht frei",
        Compare::Literal
    ));
}

/// The reason `normalise` exists, measured rather than imagined: nix broke
/// this sentence after "Abonnements" and indented the rest by seven spaces.
#[test]
fn a_pattern_spanning_a_real_line_break_still_matches() {
    let out = antwort("assertion-scope-mit-backtick.txt");
    assert!(out.contains("legt Abonnements\n       an,"));
    // Without normalising, this is what the pattern would be asked to find —
    // and it is not there. That is the whole finding, in one line.
    assert!(!out.contains("legt Abonnements an"));
    assert!(matches(&out, "legt Abonnements an", Compare::Regex));
    assert!(matches(&out, "legt Abonnements an", Compare::Literal));
}

/// A long message on ONE line, with a twenty-line stack trace above it —
/// the shape most of the 547 cases actually produce.
#[test]
fn a_real_assertion_is_found_behind_its_stack_trace() {
    let out = antwort("assertion-mehrzeilige-begruendung.txt");
    assert!(out.contains("while calling the 'head' builtin"));
    assert!(matches(&out, "den Wert STILL", Compare::Regex));
    // A pattern from a DIFFERENT case must not be found in it.
    assert!(!matches(&out, "ohne Snapshot-Abdeckung", Compare::Regex));
}

/// Exit 1 plus this output is a ruling, not a network failure. The check
/// matters because a build that died on DNS looks exactly like one that
/// died on an assertion, unless somebody keeps them apart.
#[test]
fn a_real_failed_build_is_red_and_not_network() {
    let out = antwort("assertion-scope-mit-backtick.txt");
    assert!(matches!(classify(1, &out), Outcome::Red(_)));
}

#[test]
fn a_real_green_build_is_green() {
    let out = antwort("bau-gruen.txt");
    assert!(out.contains("these 32 derivations will be built"));
    assert!(matches!(classify(0, &out), Outcome::Green));
}

/// The real `nix-instantiate --parse` failure of the one case in the
/// inventory whose lever destroys the syntax: `/nixpkgs.overlays = /d`
/// deletes the opening line and leaves the list standing.
#[test]
fn a_real_syntax_error_becomes_broken_nix() {
    let err = antwort("parse-syntaxfehler.txt");
    let msg = from_stderr(Path::new("hosts/server/gaeste/photo-01.nix"), &err)
        .expect("a syntax error must produce a verdict");
    assert_eq!(
        msg,
        "hosts/server/gaeste/photo-01.nix does not parse: \
         error: syntax error, unexpected '(', expecting 'inherit'"
    );
}

/// The other half of that rule, and the one that was wrong first: an
/// undefined variable is an EVALUATION error. The file parses.
#[test]
fn an_undefined_variable_is_not_broken_nix() {
    let err = "error: undefined variable 'pkgs'\n       at /tmp/x.nix:3:5:\n";
    assert!(from_stderr(Path::new("x.nix"), err).is_none());
}
