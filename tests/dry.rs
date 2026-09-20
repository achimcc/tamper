use std::process::Command;

use tamper::cache::Cache;
use tamper::cases::{Case, Compare, Lever};
use tamper::config::Config;
use tamper::run::{self, Ctx};
use tamper::verdict::Verdict;

/// A repo with a.nix, plus a Ctx whose config knows one target.
/// `dry` never builds, so the target attribute is never used.
fn fixture() -> (Ctx, Vec<Case>) {
    let repo = git_fixture();
    let scratch = repo.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let cfg = Config::parse(
        r#"
cases = "sabotage"
[target.server]
attr = ".#nope"
[cache]
definitions = ["a.nix"]
[lotse]
run_class = "pruefungen"
build_class = "eval"
"#,
    )
    .unwrap();
    let cache = Cache::open(&scratch.join("cache"), &repo, "nix (Nix) 2.34.0").unwrap();
    let ctx = Ctx {
        repo,
        commit: "HEAD".into(),
        scratch,
        cfg,
        cache,
        no_cache: true,
    };
    let cases = vec![
        case_with("trifft", "0,/x = 1/s//x = 2/"),
        case_with("tot", "s|gibtesnichtimtext|egal|"),
    ];
    (ctx, cases)
}

fn case_with(id: &str, sed: &str) -> Case {
    Case {
        id: id.into(),
        name: "x".into(),
        target: "server".into(),
        expect: "x".into(),
        compare: Compare::Regex,
        why: "x".into(),
        levers: vec![Lever::Sed {
            file: "a.nix".into(),
            sed: sed.into(),
        }],
    }
}

/// Same as fixture(), with one script lever added — the kind
/// hebel-pruefen.py could never see.
fn fixture_with_script_lever() -> (Ctx, Vec<Case>) {
    let (ctx, mut cases) = fixture();
    cases.push(Case {
        id: "skript".into(),
        levers: vec![Lever::Script {
            script: "printf '{ }\\n' > neu.nix".into(),
            files: vec!["neu.nix".into()],
        }],
        ..case_with("skript", "s|a|b|")
    });
    (ctx, cases)
}

fn git_fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-dry-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.nix"), "{ x = 1; }\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "init"]);
    dir
}

#[test]
fn dry_finds_the_dead_lever_and_builds_nothing() {
    // The dry probe answers the cheap half of the question in seconds:
    // "does the lever still hit?" — the half that rots silently.
    let (ctx, cases) = fixture();
    let refs: Vec<&Case> = cases.iter().collect();
    let out = run::dry(&refs, &ctx);
    let dead: Vec<&str> = out
        .iter()
        .filter(|o| o.verdict == Verdict::DeadLever)
        .map(|o| o.case_id.as_str())
        .collect();
    assert_eq!(dead, vec!["tot"]);
}

#[test]
fn dry_reaches_every_case_including_script_levers() {
    // hebel-pruefen.py reached 486 of 525 because it had to parse bash.
    // Here a lever is data, so the gap is zero — and the test says so.
    let (ctx, cases) = fixture_with_script_lever();
    let refs: Vec<&Case> = cases.iter().collect();
    let out = run::dry(&refs, &ctx);
    assert_eq!(out.len(), cases.len());
    assert!(
        out.iter()
            .any(|o| o.case_id == "skript" && o.verdict == Verdict::Ok),
        "a script lever must be judged like any other: {out:?}"
    );
}

#[test]
fn dry_never_leaves_a_worktree_behind() {
    // A probe that litters is a probe nobody runs twice.
    let (ctx, cases) = fixture();
    let refs: Vec<&Case> = cases.iter().collect();
    let _ = run::dry(&refs, &ctx);
    let out = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&ctx.repo)
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        listed.lines().count(),
        1,
        "only the repository itself may remain: {listed}"
    );
}
