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

/// Clean up after a run that died. Called once at startup.
pub fn prune(repo: &Path) -> Result<(), String> {
    let out = git(repo, &["worktree", "prune"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
