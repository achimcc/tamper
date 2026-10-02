//! One throwaway worktree per case.
//!
//! Measured on the homeserver repository (2026-09-20): `git worktree add
//! --detach` costs 0.079 s and 19 MB, against 50–90 s for one evaluation.
//! That is what makes a tree per case affordable — and it removes the whole
//! class of bugs the shell driver had to guard against: nothing is ever
//! reverted, so nothing can be left behind, and no `git checkout -- .` can
//! take somebody else's unstaged work with it.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Tree {
    pub path: PathBuf,
    repo: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git {args:?} in {}: {e}", dir.display()))
}

impl Tree {
    pub fn create(repo: &Path, commit: &str, at: &Path) -> Result<Tree, String> {
        let at = at.to_path_buf();
        let out = git(
            repo,
            &[
                "worktree",
                "add",
                "--detach",
                "--quiet",
                &at.to_string_lossy(),
                commit,
            ],
        )?;
        if !out.status.success() {
            return Err(format!(
                "cannot create worktree at {}: {}",
                at.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(Tree {
            path: at,
            repo: repo.to_path_buf(),
        })
    }

    /// Did the lever change anything? All three queries are needed: a case
    /// may change the working tree, the index, or add and remove files.
    /// With a single query, a case that unstages a file passes as "nothing
    /// changed" — which reads exactly like a check that cannot go red.
    pub fn is_dirty(&self) -> Result<bool, String> {
        let worktree = git(&self.path, &["diff", "--quiet"])?;
        let index = git(&self.path, &["diff", "--cached", "--quiet"])?;
        let status = git(&self.path, &["status", "--porcelain"])?;
        Ok(!worktree.status.success()
            || !index.status.success()
            || !String::from_utf8_lossy(&status.stdout).trim().is_empty())
    }

    /// Paths the lever touched, relative to the tree. Used to decide which
    /// files get the parse pre-check — asking git is more honest than
    /// trusting what the case declared.
    ///
    /// `-z` because a path may contain a space, and a rename carries two
    /// paths in one entry: both halves are returned, so neither escapes the
    /// parse check.
    pub fn changed_files(&self) -> Result<Vec<PathBuf>, String> {
        let out = git(&self.path, &["status", "--porcelain", "-z"])?;
        let raw = String::from_utf8_lossy(&out.stdout);
        let mut files = Vec::new();
        let mut fields = raw.split('\0').filter(|f| !f.is_empty()).peekable();
        while let Some(entry) = fields.next() {
            if entry.len() < 4 {
                continue;
            }
            let status = &entry[..2];
            files.push(PathBuf::from(&entry[3..]));
            // A rename or copy is "R  new\0old"; take the second path too.
            if (status.starts_with('R') || status.starts_with('C'))
                && let Some(origin) = fields.next()
            {
                files.push(PathBuf::from(origin));
            }
        }
        files.sort();
        files.dedup();
        Ok(files)
    }

    pub fn remove(self) -> Result<(), String> {
        let out = git(
            &self.repo,
            &[
                "worktree",
                "remove",
                "--force",
                &self.path.to_string_lossy(),
            ],
        )?;
        if !out.status.success() {
            return Err(format!(
                "cannot remove worktree {}: {}",
                self.path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }
}

/// Remove every worktree registered below `dir`, then `dir` itself, then
/// git's record of anything that is gone. This is what an interrupted run
/// calls on its own scratch directory, and what startup calls on the
/// directory of a run that died.
///
/// `git worktree prune` alone is not enough: it forgets a tree only once its
/// directory is gone, and a killed run leaves the directory behind. The
/// audit (3, CD-8) found `run-<pid>/dry-0` still listed after a SIGINT.
pub fn remove_all_under(repo: &Path, dir: &Path) -> Result<(), String> {
    let resolved = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let out = git(repo, &["worktree", "list", "--porcelain", "-z"])?;
    if !out.status.success() {
        return Err(format!(
            "cannot list worktrees: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut problems = Vec::new();
    for field in String::from_utf8_lossy(&out.stdout).split('\0') {
        let Some(path) = field.strip_prefix("worktree ") else {
            continue;
        };
        let path = PathBuf::from(path);
        if !(path.starts_with(&resolved) || path.starts_with(dir)) {
            continue;
        }
        let gone = git(
            repo,
            &["worktree", "remove", "--force", &path.to_string_lossy()],
        )?;
        // A tree whose directory has already vanished cannot be removed —
        // `prune` below forgets it. Anything else is worth saying.
        if !gone.status.success() && path.exists() {
            problems.push(format!(
                "{}: {}",
                path.display(),
                String::from_utf8_lossy(&gone.stderr).trim()
            ));
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
    }
    prune(repo)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "cannot remove worktree(s): {}",
            problems.join("; ")
        ))
    }
}

/// Remove what runs that DIED left below `base` — the `run-<pid>`
/// directories whose process no longer exists. A live run (another session
/// on the same repository) is never touched. Returns how many were removed.
pub fn sweep_dead_runs(repo: &Path, base: &Path) -> Result<usize, String> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("run-"))
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        if pid <= 0 || alive(pid) {
            continue;
        }
        remove_all_under(repo, &entry.path())?;
        removed += 1;
    }
    Ok(removed)
}

// SAFETY: the signature matches libc's `kill(pid_t, int) -> int`; `pid_t`
// and `int` are i32 on every target tamper builds for (Linux, macOS).
#[allow(unsafe_code)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Signal 0 delivers nothing and only asks whether the process exists.
/// EPERM means it does, just not ours — so only ESRCH counts as dead.
#[allow(unsafe_code)]
fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 has no effect besides the existence check.
    if unsafe { kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(3 /* ESRCH */)
}

/// Clean up after a run that died. Called once at startup.
pub fn prune(repo: &Path) -> Result<(), String> {
    let out = git(repo, &["worktree", "prune"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
