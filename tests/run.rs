use tamper::cases::{Case, Compare, Lever};
use tamper::run;

fn case(id: &str) -> Case {
    Case {
        id: id.into(),
        name: "x".into(),
        target: "server".into(),
        expect: "x".into(),
        checks: vec![],
        compare: Compare::Regex,
        why: "x".into(),
        green: false,
        levers: vec![Lever::Sed {
            file: "a.nix".into(),
            sed: "s|a|b|".into(),
        }],
    }
}

#[test]
fn the_shards_partition_the_cases_without_loss_or_overlap() {
    // sabotageproben.yml runs four lanes. If the union were not the whole
    // list, a case could go unchecked for ever and the summary would still
    // look complete.
    let cases: Vec<Case> = (0..25).map(|i| case(&i.to_string())).collect();
    let mut seen: Vec<String> = Vec::new();
    for s in 0..4 {
        for c in run::shard(&cases, s, 4) {
            seen.push(c.id.clone());
        }
    }
    seen.sort();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, 25, "a case was handed to two shards");
    assert_eq!(seen.len(), 25, "a case was handed to no shard");
}

#[test]
fn one_shard_of_one_is_everything() {
    let cases: Vec<Case> = (0..7).map(|i| case(&i.to_string())).collect();
    assert_eq!(run::shard(&cases, 0, 1).len(), 7);
}

#[test]
fn a_cache_key_that_cannot_be_computed_means_building_uncached_not_a_verdict() {
    // 2026-10-14: the key failed on a definition file the tested commit had
    // deleted, and the case came back `not-red` without a build.
    let (key, note) = run::key_or_uncached(Err(
        "hosts/vps/checks-vps.nix: no such file in the tested commit".into(),
    ));
    assert_eq!(key, None);
    let note = note.expect("the reason must be reported");
    assert!(note.contains("without the cache"), "{note}");
    assert!(note.contains("checks-vps.nix"), "{note}");
}

#[test]
fn a_computed_cache_key_is_used_and_says_nothing() {
    let (key, note) = run::key_or_uncached(Ok("abc".into()));
    assert_eq!(key.as_deref(), Some("abc"));
    assert_eq!(note, None);
}

// --- check ids in the verdict ------------------------------------------------

use tamper::build::Outcome as Build;
use tamper::verdict::Verdict;

fn case_checking(expect: &str, checks: &[&str]) -> Case {
    Case {
        expect: expect.into(),
        checks: checks.iter().map(|s| s.to_string()).collect(),
        ..case("x")
    }
}

const RED: &str = "error: Failed assertions:\n- [P-103] Pruefung 103: Gast media-01 ohne\n       Snapshot-Abdeckung\n";

#[test]
fn a_red_build_with_the_marker_is_ok() {
    let (v, detail) = run::verdict_of(&case_checking("", &["P-103"]), Build::Red(RED.into()), None);
    assert_eq!(v, Verdict::Ok);
    assert!(detail.is_empty());
}

#[test]
fn a_red_build_without_the_marker_is_other_message_and_names_it() {
    // The same ruling as a different message: the build went red, but not
    // because of the check the case names.
    let (v, detail) = run::verdict_of(&case_checking("", &["P-104"]), Build::Red(RED.into()), None);
    assert_eq!(v, Verdict::OtherMessage);
    assert!(detail.contains("[P-104]"), "{detail}");
    assert!(detail.contains("not in the output"), "{detail}");
    // and still the first error lines, as before
    assert!(detail.contains("Failed assertions"), "{detail}");
}

#[test]
fn one_of_two_markers_missing_is_other_message_naming_only_the_missing_one() {
    let (v, detail) = run::verdict_of(
        &case_checking("", &["P-103", "P-104"]),
        Build::Red(RED.into()),
        None,
    );
    assert_eq!(v, Verdict::OtherMessage);
    assert!(detail.contains("[P-104]"), "{detail}");
    assert!(
        !detail.lines().next().unwrap().contains("[P-103]"),
        "{detail}"
    );
}

#[test]
fn expect_and_checks_both_hold_is_ok_either_alone_is_not() {
    let both = case_checking("ohne Snapshot-Abdeckung", &["P-103"]);
    assert_eq!(
        run::verdict_of(&both, Build::Red(RED.into()), None).0,
        Verdict::Ok
    );

    let marker_only = case_checking("etwas ganz anderes", &["P-103"]);
    let (v, detail) = run::verdict_of(&marker_only, Build::Red(RED.into()), None);
    assert_eq!(v, Verdict::OtherMessage);
    assert!(detail.contains("`expect` did not match"), "{detail}");

    let message_only = case_checking("ohne Snapshot-Abdeckung", &["P-104"]);
    assert_eq!(
        run::verdict_of(&message_only, Build::Red(RED.into()), None).0,
        Verdict::OtherMessage
    );
}

#[test]
fn the_other_verdicts_do_not_change_for_a_case_with_checks() {
    let c = case_checking("", &["P-103"]);
    assert_eq!(run::verdict_of(&c, Build::Green, None).0, Verdict::NotRed);
    assert_eq!(
        run::verdict_of(&c, Build::Network("x".into()), None).0,
        Verdict::Network
    );
    assert_eq!(
        run::verdict_of(&c, Build::QueueTimeout, None).0,
        Verdict::QueueTimeout
    );
    // broken file, and the marker is not there: the parser spoke, not the check
    assert_eq!(
        run::verdict_of(
            &c,
            Build::Red("error: syntax error, unexpected '}'".into()),
            Some("a.nix does not parse".into())
        )
        .0,
        Verdict::BrokenNix
    );
    // broken file, but the marker IS there: the check fired anyway
    assert_eq!(
        run::verdict_of(
            &c,
            Build::Red(RED.into()),
            Some("a.nix does not parse".into())
        )
        .0,
        Verdict::Ok
    );
}

#[test]
fn the_marker_beats_the_network_guess_like_the_message_does() {
    // Case 174's rule, for ids: a marker in the output means evaluation got
    // as far as the assertion, whatever lotse's pattern says.
    let (v, _) = run::verdict_of(
        &case_checking("", &["P-103"]),
        Build::Network(RED.into()),
        None,
    );
    assert_eq!(v, Verdict::Ok);
}

#[test]
fn a_case_without_checks_reports_other_message_as_before() {
    let (v, detail) = run::verdict_of(
        &case_checking("etwas anderes", &[]),
        Build::Red(RED.into()),
        None,
    );
    assert_eq!(v, Verdict::OtherMessage);
    assert!(!detail.contains("marker"), "{detail}");
    assert!(detail.starts_with("error:"), "{detail}");
}
