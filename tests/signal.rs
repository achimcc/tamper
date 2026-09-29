//! An interrupted run leaves no worktree behind (audit 3, CD-8).
//!
//! This drives the real binary: a `dry` run whose only lever sleeps, a
//! SIGINT once the throwaway tree exists, and then the question the audit
//! asked of 0.3.1 — is the tree still on disk, and does `git worktree list`
//! still carry it? It did, both.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-signal-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with a tamper.toml and one case whose lever sleeps.
fn fixture() -> PathBuf {
    let repo = tempdir("repo");
    std::fs::create_dir_all(repo.join("sabotage")).unwrap();
    std::fs::write(repo.join("a.conf"), "x\n").unwrap();
    std::fs::write(
        repo.join("tamper.toml"),
        r#"
cases = "sabotage"
[target.server]
attr = ".#nope"
[cache]
definitions = ["a.conf"]
[lotse]
run_class = "pruefungen"
build_class = "eval"
"#,
    )
    .unwrap();
    std::fs::write(
        repo.join("sabotage/a.toml"),
        r#"
[[case]]
id = "1"
name = "schlaeft"
target = "server"
expect = "x"
why = "haelt den Baum offen, bis das Signal kommt"
levers = [ { script = "sleep 30", files = ["a.conf"] } ]
"#,
    )
    .unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "init"]);
    repo
}

/// `dry` asks for `nix-instantiate` and `nix` before the first case. Neither
/// is needed to reach the sleeping lever, and neither exists in a build
/// sandbox — so stand-ins answer `--version`.
fn stub_bin() -> PathBuf {
    let bin = tempdir("bin");
    for tool in ["nix-instantiate", "nix"] {
        let path = bin.join(tool);
        std::fs::write(&path, "#!/bin/sh\necho 'nix (Nix) 0.0-stub'\n").unwrap();
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
        std::fs::set_permissions(&path, perm).unwrap();
    }
    bin
}

fn worktrees(repo: &Path) -> usize {
    git(repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count()
}

fn interrupted_run_leaves_nothing(signal: &str) {
    let repo = fixture();
    let runtime = tempdir("runtime");
    let path = format!(
        "{}:{}",
        stub_bin().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_tamper"))
        .args(["dry", "--config", "tamper.toml"])
        .current_dir(&repo)
        .env("PATH", path)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CACHE_HOME", runtime.join("cache"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait until the throwaway tree is there — the lever is sleeping in it.
    let start = Instant::now();
    while worktrees(&repo) < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the run never created its tree"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let ok = Command::new("bash")
        .args(["-c", &format!("kill -{signal} {}", child.id())])
        .status()
        .unwrap()
        .success();
    assert!(ok);

    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "tamper did not stop on SIG{signal}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };

    assert!(!status.success(), "an interrupted run must not exit 0");
    assert_eq!(
        worktrees(&repo),
        1,
        "SIG{signal} left a worktree registered:\n{}",
        git(&repo, &["worktree", "list"])
    );
    let runs = runtime.join("tamper");
    let leftovers: Vec<_> = walk(&runs);
    assert!(
        leftovers
            .iter()
            .all(|p| !p.to_string_lossy().contains("run-")),
        "SIG{signal} left a run directory: {leftovers:?}"
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            out.push(p.clone());
            if p.is_dir() {
                out.extend(walk(&p));
            }
        }
    }
    out
}

#[test]
fn sigint_removes_the_throwaway_tree() {
    interrupted_run_leaves_nothing("INT");
}

#[test]
fn sigterm_removes_the_throwaway_tree() {
    interrupted_run_leaves_nothing("TERM");
}
