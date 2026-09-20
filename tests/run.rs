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
