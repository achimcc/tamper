//! The result cache — what turns "every case, every time" into "what changed".
//!
//! THE NAMED ASSUMPTION: a file that is neither sabotaged by this case nor
//! listed in `[cache].definitions` cannot flip its ruling.
//!
//! The counter-example that breaks it, written down so nobody has to
//! rediscover it: an assertion in `checks.nix` that aggregates over all
//! guests, plus a commit to some `hosts/server/gaeste/<x>.nix` that makes
//! the same assertion fail for a different reason. The case would still be
//! called `ok` while its message now comes from somewhere else.
//!
//! Three guards keep the assumption from rotting quietly:
//!   * only `ok` is stored — anything else always runs again;
//!   * the run reports cache COVERAGE and the age of its oldest entry,
//!     never just the number of hits;
//!   * `--no-cache` is the mode of the weekly CI run, so a stale `ok` lives
//!     at most seven days.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::cases::{Case, Lever};

pub struct Cache {
    dir: PathBuf,
    repo: PathBuf,
}

pub struct CachedOk {
    pub age_days: u64,
}

impl Cache {
    pub fn open(dir: &Path, repo: &Path) -> Result<Cache, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;
        Ok(Cache {
            dir: dir.to_path_buf(),
            repo: repo.to_path_buf(),
        })
    }

    pub fn key(&self, case: &Case, definitions: &[String]) -> Result<String, String> {
        let mut h = Sha256::new();
        h.update(b"tamper-v1\0");
        h.update(env!("CARGO_PKG_VERSION").as_bytes());
        h.update(b"\0");
        h.update(case.id.as_bytes());
        h.update(b"\0");
        h.update(case.target.as_bytes());
        h.update(b"\0");
        h.update(case.expect.as_bytes());
        h.update(b"\0");
        h.update(format!("{:?}", case.compare).as_bytes());
        h.update(b"\0");
        for lever in &case.levers {
            h.update(format!("{lever:?}").as_bytes());
            h.update(b"\0");
        }

        // The sabotaged files, then the check definitions. Both sorted, so
        // the key does not depend on the order the filesystem hands them out.
        let mut files: Vec<PathBuf> = case
            .levers
            .iter()
            .flat_map(Lever::files)
            .map(PathBuf::from)
            .collect();
        files.sort();
        files.dedup();
        for file in files {
            h.update(file.to_string_lossy().as_bytes());
            h.update(b"\0");
            // A case may sabotage a file by creating it, so "not there" is a
            // legitimate state and part of the key.
            match std::fs::read(self.repo.join(&file)) {
                Ok(bytes) => h.update(&bytes),
                Err(_) => h.update(b"<absent>"),
            }
            h.update(b"\0");
        }

        for file in expand(&self.repo, definitions)? {
            h.update(file.to_string_lossy().as_bytes());
            h.update(b"\0");
            let bytes =
                std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            h.update(&bytes);
            h.update(b"\0");
        }

        h.update(nix_version()?.as_bytes());
        Ok(format!("{:x}", h.finalize()))
    }

    pub fn get(&self, key: &str) -> Option<CachedOk> {
        let path = self.dir.join(key);
        let meta = std::fs::metadata(&path).ok()?;
        let modified = meta.modified().ok()?;
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        Some(CachedOk {
            age_days: age.as_secs() / 86_400,
        })
    }

    pub fn put_ok(&self, key: &str) -> Result<(), String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        std::fs::write(self.dir.join(key), format!("ok {stamp}\n"))
            .map_err(|e| format!("cannot write cache entry: {e}"))
    }
}

/// Turn the declared patterns into a sorted list of real files. A pattern
/// may be a plain path or end in `/**`.
pub fn expand(repo: &Path, patterns: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for pattern in patterns {
        if let Some(prefix) = pattern.strip_suffix("/**") {
            let dir = repo.join(prefix);
            if !dir.is_dir() {
                return Err(format!("{pattern}: {} is not a directory", dir.display()));
            }
            collect(&dir, &mut out)?;
        } else {
            let file = repo.join(pattern);
            if !file.is_file() {
                return Err(format!("{pattern}: no such file below {}", repo.display()));
            }
            out.push(file);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// A different Nix evaluates differently, so it belongs in the key. The CI
/// pins its version for the same reason.
fn nix_version() -> Result<String, String> {
    let out = std::process::Command::new("nix")
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run nix --version: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
