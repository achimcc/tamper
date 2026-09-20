//! Applying a lever inside a throwaway tree.
//!
//! `sed` and `perl` are run WITHOUT a shell: the expression is one argv
//! element, so nothing has to be quoted and nothing can be re-interpreted.
//! That removes the failure that cost the palette case on 2026-09-03 — a raw
//! `"` inserted into a Nix string that was itself inside `"`.

use std::path::Path;
use std::process::Command;

use crate::cases::Lever;

pub fn apply(lever: &Lever, tree: &Path) -> Result<(), String> {
    match lever {
        Lever::Sed { file, sed } => {
            require(tree, file)?;
            run(Command::new("sed")
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

fn require(tree: &Path, file: &str) -> Result<(), String> {
    if tree.join(file).exists() {
        Ok(())
    } else {
        Err(format!(
            "lever points at {file}, which does not exist in the tree"
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
