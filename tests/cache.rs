use std::path::{Path, PathBuf};
use std::process::Command;

use tamper::cache::Cache;
use tamper::cases::{Case, Compare, Lever};

const NIX: &str = "nix (Nix) 2.34.0";

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Commit whatever is in the working tree and return the new sha.
fn commit(repo: &Path) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "--quiet", "--allow-empty", "-m", "x"]);
    git(repo, &["rev-parse", "HEAD"])
}

/// A throwaway repository with one commit. The key is read from a COMMIT, so
/// the fixture must be a real repository, not a directory of files.
fn sandbox() -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!(
        "tamper-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(repo.join("checks.nix"), "assertions\n").unwrap();
    std::fs::write(repo.join("lib/gaeste.nix"), "{ uidBasis = 200000; }\n").unwrap();
    std::fs::write(repo.join("lib/wirte.nix"), "{ }\n").unwrap();
    std::fs::write(repo.join("README.md"), "prose\n").unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    let sha = commit(&repo);
    (dir, sha)
}

fn open(dir: &Path, sha: &str) -> Cache {
    Cache::open(&dir.join("cache"), &dir.join("repo"), sha, NIX).unwrap()
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
fn the_same_commit_gives_the_same_key() {
    let (dir, sha) = sandbox();
    let a = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    let b = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn changing_the_sabotaged_file_changes_the_key() {
    let (dir, sha) = sandbox();
    let before = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/lib/gaeste.nix"), "{ uidBasis = 300000; }\n").unwrap();
    let after = commit(&dir.join("repo"));
    assert_ne!(before, open(&dir, &after).key(&a_case(), &defs()).unwrap());
}

#[test]
fn changing_the_check_definitions_changes_the_key() {
    let (dir, sha) = sandbox();
    let before = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/checks.nix"), "more assertions\n").unwrap();
    let after = commit(&dir.join("repo"));
    assert_ne!(before, open(&dir, &after).key(&a_case(), &defs()).unwrap());
}

#[test]
fn the_working_tree_does_not_change_the_key_of_a_commit() {
    // THE BUG THIS REPLACES (found 2026-09-20 during the homeserver
    // acceptance): the key read its files from the working tree while the
    // case was built from `--commit`. Merging `origin/main` mid-run would have
    // stored an `ok` under a key describing a `checks.nix` that was never
    // built. What is built is the commit, so the commit is what the key reads.
    let (dir, sha) = sandbox();
    let before = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/checks.nix"), "uncommitted assertions\n").unwrap();
    std::fs::write(dir.join("repo/lib/gaeste.nix"), "{ uncommitted = 1; }\n").unwrap();
    assert_eq!(before, open(&dir, &sha).key(&a_case(), &defs()).unwrap());
}

#[test]
fn changing_the_expected_pattern_changes_the_key() {
    let (dir, sha) = sandbox();
    let c = open(&dir, &sha);
    let before = c.key(&a_case(), &defs()).unwrap();
    let mut other = a_case();
    other.expect = "etwas ganz anderes".into();
    assert_ne!(before, c.key(&other, &defs()).unwrap());
}

#[test]
fn a_file_the_case_creates_is_absent_and_that_is_part_of_the_key() {
    // A case may sabotage by CREATING a file (case 199 writes a new .nix).
    let (dir, sha) = sandbox();
    let c = open(&dir, &sha);
    let mut creates = a_case();
    creates.levers = vec![Lever::Sed {
        file: "gibt-es-noch-nicht.nix".into(),
        sed: "s|a|b|".into(),
    }];
    let absent = c.key(&creates, &defs()).unwrap();
    std::fs::write(dir.join("repo/gibt-es-noch-nicht.nix"), "{ }\n").unwrap();
    let after = commit(&dir.join("repo"));
    assert_ne!(absent, open(&dir, &after).key(&creates, &defs()).unwrap());
}

#[test]
fn prose_outside_the_declared_set_does_not_change_the_key() {
    // THE NAMED ASSUMPTION. This test exists so that anybody widening it
    // has to come here and say so out loud.
    let (dir, sha) = sandbox();
    let before = open(&dir, &sha).key(&a_case(), &defs()).unwrap();
    std::fs::write(dir.join("repo/README.md"), "different prose\n").unwrap();
    let after = commit(&dir.join("repo"));
    assert_eq!(before, open(&dir, &after).key(&a_case(), &defs()).unwrap());
}

#[test]
fn a_glob_covers_every_file_below_it() {
    let (dir, sha) = sandbox();
    let c = open(&dir, &sha);
    let names = c.expand(&defs()).unwrap();
    assert!(names.iter().any(|n| n == "lib/gaeste.nix"), "{names:?}");
    assert!(names.iter().any(|n| n == "lib/wirte.nix"), "{names:?}");
    assert!(names.iter().any(|n| n == "checks.nix"), "{names:?}");
    assert!(!names.iter().any(|n| n == "README.md"), "{names:?}");
}

#[test]
fn a_missing_definition_file_is_an_error_and_not_a_silent_skip() {
    // A cache key that quietly drops a file it cannot find is a cache that
    // goes stale without saying so.
    let (dir, sha) = sandbox();
    let err = open(&dir, &sha)
        .expand(&["gibtsnicht.nix".into()])
        .unwrap_err();
    assert!(err.contains("gibtsnicht.nix"), "{err}");
    let err = open(&dir, &sha)
        .expand(&["gibtsnicht/**".into()])
        .unwrap_err();
    assert!(err.contains("gibtsnicht"), "{err}");
}

#[test]
fn a_definition_only_in_the_working_tree_is_missing() {
    // The mirror image of the bug: a file that exists on disk but not in the
    // commit is not part of what gets built, so it cannot define a check.
    let (dir, sha) = sandbox();
    std::fs::write(dir.join("repo/neu.nix"), "{ }\n").unwrap();
    assert!(open(&dir, &sha).expand(&["neu.nix".into()]).is_err());
}

#[test]
fn an_unknown_commit_is_an_error() {
    let (dir, _) = sandbox();
    let err = Cache::open(&dir.join("cache"), &dir.join("repo"), "0000000", NIX)
        .err()
        .expect("an unknown commit must not open a cache");
    assert!(err.contains("0000000"), "{err}");
}

#[test]
fn only_ok_is_stored_and_it_comes_back_with_an_age() {
    let (dir, sha) = sandbox();
    let c = open(&dir, &sha);
    let key = c.key(&a_case(), &defs()).unwrap();
    assert!(c.get(&key).is_none());
    c.put_ok(&key).unwrap();
    let hit = c.get(&key).expect("an ok must come back");
    assert_eq!(hit.age_days, 0);
}

