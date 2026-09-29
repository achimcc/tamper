use std::process::Command;

use tamper::tree::{self, Tree};

/// A throwaway git repository with one commit.
fn fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-tree-{}-{}",
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
    std::fs::write(dir.join("b.nix"), "{ y = 2; }\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "init"]);
    dir
}

#[test]
fn a_fresh_tree_is_clean() {
    let repo = fixture();
    let at = repo.join("../wt-clean");
    let tree = Tree::create(&repo, "HEAD", &at).unwrap();
    assert!(!tree.is_dirty().unwrap());
    assert!(tree.changed_files().unwrap().is_empty());
    tree.remove().unwrap();
}

#[test]
fn a_changed_file_is_dirty() {
    let repo = fixture();
    let tree = Tree::create(&repo, "HEAD", &repo.join("../wt-dirty")).unwrap();
    std::fs::write(tree.path.join("a.nix"), "{ x = 2; }\n").unwrap();
    assert!(tree.is_dirty().unwrap());
    assert_eq!(
        tree.changed_files().unwrap(),
        vec![std::path::PathBuf::from("a.nix")]
    );
    tree.remove().unwrap();
}

#[test]
fn a_file_taken_out_of_the_index_is_dirty() {
    // THIS is why all three queries are needed. Case 39 removes
    // scripts/inventar.env from the index; `git diff` alone sees nothing and
    // the case would be reported as a dead lever.
    let repo = fixture();
    let tree = Tree::create(&repo, "HEAD", &repo.join("../wt-index")).unwrap();
    let ok = Command::new("git")
        .args(["rm", "--cached", "--quiet", "a.nix"])
        .current_dir(&tree.path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    assert!(
        tree.is_dirty().unwrap(),
        "a staged deletion must count as dirty"
    );
    tree.remove().unwrap();
}

#[test]
fn a_new_untracked_file_is_dirty() {
    // Case 93 adds a sabotage blueprint; `git diff` does not see it either.
    let repo = fixture();
    let tree = Tree::create(&repo, "HEAD", &repo.join("../wt-new")).unwrap();
    std::fs::write(tree.path.join("c.nix"), "{ }\n").unwrap();
    assert!(tree.is_dirty().unwrap());
    assert!(
        tree.changed_files()
            .unwrap()
            .contains(&std::path::PathBuf::from("c.nix"))
    );
    tree.remove().unwrap();
}

#[test]
fn removing_a_tree_leaves_the_repository_without_it() {
    let repo = fixture();
    let at = repo.join("../wt-gone");
    let tree = Tree::create(&repo, "HEAD", &at).unwrap();
    let path = tree.path.clone();
    tree.remove().unwrap();
    assert!(!path.exists());
    tree::prune(&repo).unwrap();
}

// --- nothing is left behind by a run that died (audit 3, CD-8) -------------

fn listed(repo: &std::path::Path) -> String {
    let out = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(repo)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The pid of a process that has certainly exited.
fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
fn the_trees_of_a_dead_run_are_removed_and_deregistered() {
    // A run killed mid-case leaves `run-<pid>/slot-0` on disk AND in
    // `git worktree list`. `git worktree prune` alone keeps both, because
    // the directory still exists — the audit found exactly that entry.
    let repo = fixture();
    let base = repo.with_extension("scratch");
    let dead = base.join(format!("run-{}", dead_pid()));
    let alive = base.join(format!("run-{}", std::process::id()));
    let left = Tree::create(&repo, "HEAD", &dead.join("slot-0")).unwrap();
    let mine = Tree::create(&repo, "HEAD", &alive.join("slot-0")).unwrap();
    assert_eq!(listed(&repo).lines().count(), 3);

    let removed = tree::sweep_dead_runs(&repo, &base).unwrap();

    assert_eq!(removed, 1);
    assert!(!left.path.exists(), "the dead run's tree is still on disk");
    assert!(!dead.exists(), "the dead run's directory is still there");
    assert!(
        mine.path.exists(),
        "a LIVE run's tree must never be touched"
    );
    let now = listed(&repo);
    assert_eq!(now.lines().count(), 2, "{now}");
    assert!(!now.contains(&dead.to_string_lossy().into_owned()), "{now}");
    mine.remove().unwrap();
}

#[test]
fn removing_everything_under_a_run_directory_deregisters_it() {
    let repo = fixture();
    let run = repo.with_extension("scratch-alle").join("run-1");
    let _a = Tree::create(&repo, "HEAD", &run.join("slot-0")).unwrap();
    let _b = Tree::create(&repo, "HEAD", &run.join("slot-1")).unwrap();
    tree::remove_all_under(&repo, &run).unwrap();
    assert!(!run.exists());
    assert_eq!(listed(&repo).lines().count(), 1, "{}", listed(&repo));
}
