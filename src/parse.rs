//! Does the sabotaged file still parse?
//!
//! Without this question, a lever that destroys the syntax proves only that
//! broken Nix does not build — the parser fails BEFORE any assertion runs,
//! and the case reads as "fell, but with a different message". That is the
//! palette case from 2026-09-03, and it is invisible in the shell driver.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn check(tree: &Path, files: &[PathBuf]) -> Result<Option<String>, String> {
    for file in files {
        if file.extension().is_none_or(|e| e != "nix") {
            continue;
        }
        let full = tree.join(file);
        if !full.exists() {
            continue;
        }
        let out = Command::new("nix-instantiate")
            .arg("--parse")
            .arg(&full)
            .output()
            .map_err(|e| format!("cannot run nix-instantiate: {e}"))?;
        if !out.status.success() {
            let first = String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            return Ok(Some(format!("{} does not parse: {first}", file.display())));
        }
    }
    Ok(None)
}
