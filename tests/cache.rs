use tamper::cache::{self, Cache};
use tamper::cases::{Case, Compare, Lever};

fn sandbox() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("repo/lib")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("repo/checks.nix"), "assertions\n").unwrap();
    std::fs::write(dir.join("repo/lib/gaeste.nix"), "{ uidBasis = 200000; }\n").unwrap();
    std::fs::write(dir.join("repo/lib/wirte.nix"), "{ }\n").unwrap();
    std::fs::write(dir.join("repo/README.md"), "prose\n").unwrap();
    dir
}

fn a_case() -> Case {
    Case {
        id: "9c".into(),
        name: "x".into(),
        target: "server".into(),
        expect: "ueber einem eigenen Mount".into(),
        compare: Compare::Regex,
        why: "x".into(),
        green: false,
        levers: vec![Lever::Sed {
            file: "lib/gaeste.nix".into(),
            sed: "s|a|b|".into(),
        }],
    }
}

fn defs() -> Vec<String> {
    vec!["checks.nix".into(), "lib/**".into()]
}

#[test]
fn the_same_tree_gives_the_same_key() {
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let a = c.key(&a_case(), &defs()).unwrap();
    let b = c.key(&a_case(), &defs()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn changing_the_sabotaged_file_changes_the_key() {
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let before = c.key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/lib/gaeste.nix"), "{ uidBasis = 300000; }\n").unwrap();
    assert_ne!(before, c.key(&a_case(), &defs()).unwrap());
}

#[test]
fn changing_the_check_definitions_changes_the_key() {
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let before = c.key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/checks.nix"), "more assertions\n").unwrap();
    assert_ne!(before, c.key(&a_case(), &defs()).unwrap());
}

#[test]
fn changing_the_expected_pattern_changes_the_key() {
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let before = c.key(&a_case(), &defs()).unwrap();
    let mut other = a_case();
    other.expect = "etwas ganz anderes".into();
    assert_ne!(before, c.key(&other, &defs()).unwrap());
}

#[test]
fn prose_outside_the_declared_set_does_not_change_the_key() {
    // THE NAMED ASSUMPTION. This test exists so that anybody widening it
    // has to come here and say so out loud.
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let before = c.key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/README.md"), "different prose\n").unwrap();
    assert_eq!(before, c.key(&a_case(), &defs()).unwrap());
}

#[test]
fn a_glob_covers_every_file_below_it() {
    let dir = sandbox();
    let files = cache::expand(&dir.join("repo"), &defs()).unwrap();
    let names: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("lib/gaeste.nix")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("lib/wirte.nix")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.ends_with("checks.nix")), "{names:?}");
    assert!(!names.iter().any(|n| n.ends_with("README.md")), "{names:?}");
}

#[test]
fn a_missing_definition_file_is_an_error_and_not_a_silent_skip() {
    // A cache key that quietly drops a file it cannot find is a cache that
    // goes stale without saying so.
    let dir = sandbox();
    let err = cache::expand(&dir.join("repo"), &["gibtsnicht.nix".into()]).unwrap_err();
    assert!(err.contains("gibtsnicht.nix"), "{err}");
}

#[test]
fn only_ok_is_stored_and_it_comes_back_with_an_age() {
    let dir = sandbox();
    let c = Cache::open(&dir.join("cache"), &dir.join("repo"), "nix (Nix) 2.34.0").unwrap();
    let key = c.key(&a_case(), &defs()).unwrap();
    assert!(c.get(&key).is_none());
    c.put_ok(&key).unwrap();
    let hit = c.get(&key).expect("an ok must come back");
    assert_eq!(hit.age_days, 0);
}
