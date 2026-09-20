use tamper::verdict::Verdict;

#[test]
fn every_verdict_has_a_distinct_slug() {
    let mut slugs: Vec<&str> = Verdict::ALL.iter().map(|v| v.slug()).collect();
    slugs.sort_unstable();
    let count = slugs.len();
    slugs.dedup();
    assert_eq!(slugs.len(), count, "two verdicts share a slug");
    assert_eq!(count, 8);
}

#[test]
fn network_and_queue_timeout_are_not_rulings() {
    // They say nothing about the tree, so they must not count as findings
    // and must not count as clean either.
    assert!(!Verdict::Network.is_ruling());
    assert!(!Verdict::QueueTimeout.is_ruling());
    assert!(!Verdict::Network.is_finding());
    assert!(!Verdict::QueueTimeout.is_finding());
}

#[test]
fn the_four_findings_are_findings() {
    for v in [
        Verdict::DeadLever,
        Verdict::NotRed,
        Verdict::OtherMessage,
        Verdict::BrokenNix,
        Verdict::FalseAlarm,
    ] {
        assert!(v.is_finding(), "{} must be a finding", v.slug());
        assert!(v.is_ruling());
    }
    assert!(Verdict::Ok.is_ruling());
    assert!(!Verdict::Ok.is_finding());
}

#[test]
fn every_verdict_explains_itself() {
    for v in Verdict::ALL {
        assert!(!v.explain().is_empty(), "{} has no explanation", v.slug());
    }
}

#[test]
fn a_false_alarm_is_the_opposite_finding_of_not_red() {
    // A case may state that a change must leave the check GREEN. If the
    // check fires anyway, that is a finding — but the opposite one from
    // "the check does not fire", and calling it `not-red` would say the
    // reverse of what happened.
    assert_ne!(Verdict::FalseAlarm, Verdict::NotRed);
    assert!(Verdict::FalseAlarm.is_finding());
    assert!(Verdict::FalseAlarm.is_ruling());
    assert!(Verdict::FalseAlarm.explain().contains("green"));
}
