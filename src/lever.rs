//! Applying a lever inside a throwaway tree.
//!
//! `sed` and `perl` are run WITHOUT a shell: the expression is one argv
//! element, so nothing has to be quoted and nothing can be re-interpreted.
//! That removes the failure that cost the palette case on 2026-09-03 — a raw
//! `"` inserted into a Nix string that was itself inside `"`.
//!
//! "Without a shell" is about QUOTING, not about trust. Two more fences keep
//! a lever inside the tree it is handed (audit 3, CD-8, 2026-09-27):
//!   * the file must resolve to a path INSIDE the tree — no absolute path,
//!     no `..`, no symlink that points out. Up to 0.3.1 each of those edited
//!     a file outside, and `tamper dry` still called the case `ok`;
//!   * `sed` runs with `--sandbox`, which refuses the `e`, `r` and `w`
//!     commands: `e` starts a shell after all, `r`/`w` reach any file.
//!
//! `perl -e` and `script` levers stay what they are — code from the case
//! file, trusted like the repository they live in.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cases::{self, Lever};

pub fn apply(lever: &Lever, tree: &Path) -> Result<(), String> {
    match lever {
        Lever::Sed { file, sed } => {
            require(tree, file)?;
            run(Command::new("sed")
                .arg("--sandbox")
                .arg("-i")
                .arg(sed)
                .arg(file)
                .current_dir(tree))
        }
        Lever::Perl { file, perl } => {
            require(tree, file)?;
            run(Command::new("perl")
                .arg("-0pi")
                .arg("-e")
                .arg(perl)
                .arg(file)
                .current_dir(tree))
        }
        Lever::Script { script, .. } => run(Command::new("bash")
            .arg("-euo")
            .arg("pipefail")
            .arg("-c")
            .arg(script)
            .current_dir(tree)),
    }
}

/// The file must exist, and it must be the tree's own: the check is made on
/// the RESOLVED path, because a symlink is lexically harmless and really
/// outside.
fn require(tree: &Path, file: &str) -> Result<PathBuf, String> {
    cases::check_lever_path(file)?;
    let resolved = tree
        .join(file)
        .canonicalize()
        .map_err(|_| format!("lever points at {file}, which does not exist in the tree"))?;
    let root = tree
        .canonicalize()
        .map_err(|e| format!("cannot resolve the tree {}: {e}", tree.display()))?;
    if resolved.starts_with(&root) {
        Ok(resolved)
    } else {
        Err(format!(
            "lever points at {file}, which resolves to {} — outside the throwaway tree",
            resolved.display()
        ))
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("cannot run lever: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "lever failed (exit {}): {}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}
