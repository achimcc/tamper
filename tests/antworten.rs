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

use tamper::build::{Outcome, classify};
use tamper::cases::{Case, Compare, Lever};
use tamper::message::matches;
use tamper::parse::from_stderr;
use tamper::run::verdict_of;
use tamper::verdict::Verdict;

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
    assert!(matches(
        &out,
        "gibt den .email.-Scope nicht frei",
        Compare::Regex
    ));
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

/// The router cases compare LITERALLY (`compare = "literal"`), because their
/// patterns carry `[`, `]` and `(` from the probe's own output. This is one
/// of them, from a real QEMU run of the OpenWrt image.
#[test]
fn a_real_router_probe_failure_matches_literally() {
    let out = antwort("router-probe-fw4.txt");
    assert!(out.contains("PROBE-NEIN: fw4 check (Exit 0)"));
    assert!(matches(&out, "PROBE-NEIN: fw4 check", Compare::Literal));
    assert!(matches!(classify(1, &out), Outcome::Red(_)));
}

/// AND THE LIMIT THAT ONLY REAL OUTPUT SHOWS: nix prefixes every line a
/// builder writes with `> `. Normalising whitespace does not remove it, so a
/// pattern spanning two builder lines cannot match — the `>` lands in the
/// middle of it. No case in the inventory depends on that today; the test is
/// here so the next person finds the answer instead of the puzzle.
#[test]
fn the_builder_prefix_survives_normalising() {
    let out = antwort("router-probe-fw4.txt");
    assert!(out.contains("> router-probe: die VM meldet nicht gruen:\n       > PROBE-NEIN"));
    // Across the break the text reads "…gruen: > PROBE-NEIN…", not "…gruen: PROBE-NEIN…".
    assert!(matches(
        &out,
        "die VM meldet nicht gruen: > PROBE-NEIN",
        Compare::Literal
    ));
    assert!(!matches(
        &out,
        "die VM meldet nicht gruen: PROBE-NEIN",
        Compare::Literal
    ));
}

/// The other half of that rule, and the one that was wrong first: an
/// undefined variable is an EVALUATION error. The file parses.
#[test]
fn an_undefined_variable_is_not_broken_nix() {
    let err = "error: undefined variable 'pkgs'\n       at /tmp/x.nix:3:5:\n";
    assert!(from_stderr(Path::new("x.nix"), err).is_none());
}

fn case_expecting(expect: &str) -> Case {
    Case {
        id: "x".into(),
        name: "x".into(),
        target: "server".into(),
        expect: expect.into(),
        compare: Compare::Regex,
        why: "x".into(),
        green: false,
        levers: vec![Lever::Sed {
            file: "lib/gaeste.nix".into(),
            sed: "s|a|b|".into(),
        }],
    }
}

/// CASE 174, THE WHOLE LOTSE LOG, UNCUT (2026-09-22). The assertion the case
/// waits for quotes the nftables error it guards against — "Could not
/// resolve hostname" — and lotse's retry pattern took that quotation for a
/// DNS failure: four attempts, exit 201. The expected message is in every
/// one of them. Unlike the other files here, the lotse lines are kept: they
/// ARE the finding.
#[test]
fn case_174_the_expected_message_beats_the_network_guess() {
    let out = antwort("assertion-zitiert-dns-fehler.txt");
    assert!(out.contains("lotse: exit=201 attempts=4"));
    assert_eq!(out.matches("Could not resolve hostname").count(), 4);

    // What lotse told us: the network.
    let build = classify(201, &out);
    assert!(matches!(build, Outcome::Network(_)));
    // What it was: the check fired.
    let (v, _) = verdict_of(
        &case_expecting("koennen kein drittes Oktett sein"),
        build,
        None,
    );
    assert_eq!(v, Verdict::Ok);
}

/// The same output with a pattern that is NOT in it stays without a ruling —
/// the rule is "the expected message is there", not "lotse is ignored".
#[test]
fn case_174_output_without_the_expected_message_stays_network() {
    let out = antwort("assertion-zitiert-dns-fehler.txt");
    let (v, _) = verdict_of(
        &case_expecting("ohne Snapshot-Abdeckung"),
        classify(201, &out),
        None,
    );
    assert_eq!(v, Verdict::Network);
}

/// And tamper's OWN pattern no longer makes the same mistake: had this
/// output reached us with nix's raw exit code 1, "Could not resolve host"
/// without the colon would have called it the network too.
#[test]
fn case_174_raw_exit_1_is_red_not_network() {
    let out = antwort("assertion-zitiert-dns-fehler.txt");
    assert!(matches!(classify(1, &out), Outcome::Red(_)));
}

/// CASE 199: a file that does not parse, and a check that fired anyway —
/// because it reads the file with `grep` and never evaluates it. Judged
/// before the build this was `broken-nix`; the check works.
#[test]
fn a_check_that_fires_despite_a_syntax_error_is_ok() {
    let parse = from_stderr(
        Path::new("hosts/server/gaeste/photo-01.nix"),
        &antwort("parse-syntaxfehler.txt"),
    );
    assert!(parse.is_some());
    let out = antwort("assertion-scope-mit-backtick.txt");
    let (v, _) = verdict_of(
        &case_expecting("gibt den .email.-Scope nicht frei"),
        classify(1, &out),
        parse,
    );
    assert_eq!(v, Verdict::Ok);
}

/// THE PALETTE CASE stays what it was: the file does not parse and the build
/// failed with something else — the parser, not the assertion.
#[test]
fn a_syntax_error_with_another_message_is_broken_nix() {
    let parse = from_stderr(
        Path::new("hosts/server/gaeste/photo-01.nix"),
        &antwort("parse-syntaxfehler.txt"),
    );
    let out = antwort("assertion-scope-mit-backtick.txt");
    let (v, detail) = verdict_of(
        &case_expecting("onnxruntime ohne OpenVINO"),
        classify(1, &out),
        parse,
    );
    assert_eq!(v, Verdict::BrokenNix);
    assert!(detail.contains("does not parse"), "{detail}");
}

/// Without a syntax error, a different message is still `other-message`.
#[test]
fn another_message_without_a_syntax_error_is_other_message() {
    let out = antwort("assertion-scope-mit-backtick.txt");
    let (v, _) = verdict_of(
        &case_expecting("etwas ganz anderes"),
        classify(1, &out),
        None,
    );
    assert_eq!(v, Verdict::OtherMessage);
}
