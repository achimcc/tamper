use tamper::report::{Blocked, Summary};
use tamper::run::Outcome;
use tamper::verdict::Verdict;

fn outcome(id: &str, v: Verdict) -> Outcome {
    Outcome {
        case_id: id.into(),
        verdict: v,
        detail: String::new(),
        from_cache: false,
        cache_age_days: None,
    }
}

fn cached(id: &str, age_days: u64) -> Outcome {
    Outcome {
        case_id: id.into(),
        verdict: Verdict::Ok,
        detail: String::new(),
        from_cache: true,
        cache_age_days: Some(age_days),
    }
}

#[test]
fn all_ok_is_exit_zero() {
    let s = Summary::of(&[outcome("1", Verdict::Ok)]);
    assert_eq!(s.exit_code(), 0);
}

#[test]
fn a_finding_is_exit_one() {
    let s = Summary::of(&[outcome("1", Verdict::Ok), outcome("2", Verdict::DeadLever)]);
    assert_eq!(s.exit_code(), 1);
}

#[test]
fn a_case_without_a_ruling_is_exit_four_and_not_zero() {
    // The whole point: "the network was gone" must never read as "all good".
    let s = Summary::of(&[outcome("1", Verdict::Ok), outcome("2", Verdict::Network)]);
    assert_eq!(s.exit_code(), 4);
}

#[test]
fn a_finding_outranks_a_missing_ruling() {
    let s = Summary::of(&[
        outcome("1", Verdict::NotRed),
        outcome("2", Verdict::Network),
    ]);
    assert_eq!(s.exit_code(), 1);
}

#[test]
fn the_report_names_the_cache_coverage_not_the_hit_count() {
    // The lesson from blueprints-pruefen.py: a check that does not reach all
    // of its subjects must name the GAP, not the number of hits.
    let s = Summary::of(&[cached("1", 2), cached("2", 6), outcome("3", Verdict::Ok)]);
    let text = s.render();
    assert!(text.contains("2 of 3"), "{text}");
    assert!(text.contains("6 day"), "{text}");
}

#[test]
fn a_run_without_the_cache_says_so_instead_of_claiming_zero_days() {
    let s = Summary::of(&[outcome("1", Verdict::Ok)]);
    let text = s.render();
    assert!(text.contains("0 of 1"), "{text}");
    assert!(!text.contains("oldest"), "{text}");
}

fn blocked(target: &str, cases: usize) -> Blocked {
    Blocked {
        target: target.into(),
        reason: "Failed assertions:\n- feed hash mismatch".into(),
        cases,
    }
}

/// THE HOMESERVER ACCEPTANCE, 2026-09-20: OpenWrt rebuilt its package feeds,
/// the router baseline went red on a hash, and the whole run of 540 cases
/// stopped with exit 3 — ten healthy targets included. A red baseline now
/// blocks its own target and nothing else, and it still cannot pass as 0.
#[test]
fn a_red_baseline_of_one_target_is_exit_three_while_the_others_ran() {
    let s = Summary::of(&[outcome("1", Verdict::Ok), outcome("2", Verdict::Ok)])
        .with_blocked(vec![blocked("router-probe", 6)]);
    assert_eq!(s.ok, 2);
    assert_eq!(s.exit_code(), 3);
}

#[test]
fn a_finding_outranks_a_red_baseline() {
    // The same order as finding-before-missing-ruling: the report names
    // both, the exit code names the one a human must act on first.
    let s = Summary::of(&[outcome("1", Verdict::NotRed)])
        .with_blocked(vec![blocked("router-probe", 6)]);
    assert_eq!(s.exit_code(), 1);
}

#[test]
fn a_red_baseline_outranks_a_missing_ruling() {
    let s = Summary::of(&[outcome("1", Verdict::Network)])
        .with_blocked(vec![blocked("router-probe", 6)]);
    assert_eq!(s.exit_code(), 3);
}

#[test]
fn the_report_names_the_blocked_target_its_cases_and_why() {
    let s =
        Summary::of(&[outcome("1", Verdict::Ok)]).with_blocked(vec![blocked("router-probe", 6)]);
    let text = s.render();
    assert!(text.contains("router-probe"), "{text}");
    assert!(text.contains("6 case(s)"), "{text}");
    assert!(text.contains("feed hash mismatch"), "{text}");
    // And the tally must not pretend those six were counted.
    assert!(text.contains("1 of 1 cases ok"), "{text}");
}

#[test]
fn without_a_blocked_target_the_report_says_nothing_about_baselines() {
    let text = Summary::of(&[outcome("1", Verdict::Ok)]).render();
    assert!(!text.contains("BASELINE"), "{text}");
}
