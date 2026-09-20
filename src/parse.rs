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
            let stderr = String::from_utf8_lossy(&out.stderr);
            // ONLY A SYNTAX ERROR COUNTS. Measured 2026-09-20:
            // `nix-instantiate --parse` resolves variables inside string
            // interpolations, so a syntactically perfect file with a free
            // `pkgs` fails with "undefined variable". That file parses; it
            // just cannot be evaluated on its own, and a case may create
            // exactly such a snippet on purpose for a check that only reads
            // source text. Calling it broken-nix would be this verdict's own
            // misattribution, pointing the other way.
            if !stderr.contains("syntax error") {
                continue;
            }
            let first = stderr.lines().next().unwrap_or("").trim().to_string();
            return Ok(Some(format!("{} does not parse: {first}", file.display())));
        }
    }
    Ok(None)
}
