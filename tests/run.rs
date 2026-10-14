use tamper::cases::{Case, Compare, Lever};
use tamper::run;

fn case(id: &str) -> Case {
    Case {
        id: id.into(),
        name: "x".into(),
        target: "server".into(),
        expect: "x".into(),
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
