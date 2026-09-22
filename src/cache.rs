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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::cases::{Case, Lever};

pub struct Cache {
    dir: PathBuf,
    /// Every file of the tested commit, path -> blob id.
    ///
    /// THE KEY IS READ FROM THE COMMIT, NOT FROM THE WORKING TREE. What gets
    /// built is `--commit` (default HEAD), checked out into a throwaway tree;
    /// the working tree takes no part in it. Up to 0.2.1 the key read the
    /// working tree, which is the same thing only while both carry the same
    /// files. During the homeserver acceptance (2026-09-20) the run was pinned
    /// to one commit for days while the branch could move on: a merge would
    /// have stored an `ok` under a key describing a `checks.nix` that was
    /// never built.
    ///
    /// Git's blob ids are content hashes, so hashing the id is hashing the
    /// content — and one `git ls-tree` answers every case of the run.
    files: BTreeMap<String, String>,
    /// A different Nix evaluates differently, so its version belongs in
    /// every key. It is MEASURED ONCE by the caller and handed in — asking
    /// per case would start 525 processes to get the same answer 525 times,
    /// and it would make the cache unusable anywhere `nix` is absent, such
    /// as inside a build sandbox running these very tests.
    nix_version: String,
}

pub struct CachedOk {
    pub age_days: u64,
}

impl Cache {
    pub fn open(dir: &Path, repo: &Path, commit: &str, nix_version: &str) -> Result<Cache, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;
        Ok(Cache {
            dir: dir.to_path_buf(),
            files: list_commit(repo, commit)?,
            nix_version: nix_version.to_string(),
        })
    }

    pub fn key(&self, case: &Case, definitions: &[String]) -> Result<String, String> {
        let mut h = Sha256::new();
        h.update(b"tamper-v2\0");
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
        // the key does not depend on the order they are handed out.
        let mut files: Vec<String> = case
            .levers
            .iter()
            .flat_map(Lever::files)
            .map(|f| f.to_string())
            .collect();
        files.sort();
        files.dedup();
        for file in files {
            h.update(file.as_bytes());
            h.update(b"\0");
            // A case may sabotage a file by creating it, so "not there" is a
            // legitimate state and part of the key.
            match self.files.get(&file) {
                Some(blob) => h.update(blob.as_bytes()),
                None => h.update(b"<absent>"),
            }
            h.update(b"\0");
        }

        for file in self.expand(definitions)? {
            h.update(file.as_bytes());
            h.update(b"\0");
            h.update(self.files[&file].as_bytes());
            h.update(b"\0");
        }

        h.update(self.nix_version.as_bytes());
        Ok(format!("{:x}", h.finalize()))
    }

    /// Turn the declared patterns into a sorted list of files OF THE COMMIT.
    /// A pattern may be a plain path or end in `/**`. A pattern that matches
    /// nothing is an error: a key that quietly drops a file goes stale
    /// without saying so.
    pub fn expand(&self, patterns: &[String]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for pattern in patterns {
            if let Some(prefix) = pattern.strip_suffix("/**") {
                let prefix = format!("{}/", prefix.trim_end_matches('/'));
                let before = out.len();
                out.extend(
                    self.files
                        .keys()
                        .filter(|f| f.starts_with(&prefix))
                        .cloned(),
                );
                if out.len() == before {
                    return Err(format!("{pattern}: no file below it in the tested commit"));
                }
            } else if self.files.contains_key(pattern) {
                out.push(pattern.clone());
            } else {
                return Err(format!("{pattern}: no such file in the tested commit"));
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
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

/// Every file of `commit`, path -> blob id, from one `git ls-tree`.
/// `-z` so that no path with an odd character can split an entry.
fn list_commit(repo: &Path, commit: &str) -> Result<BTreeMap<String, String>, String> {
    let out = Command::new("git")
        .args(["ls-tree", "-r", "-z", "--full-tree", commit])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("cannot run git ls-tree: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cannot list commit {commit}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut files = BTreeMap::new();
    for entry in out.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8_lossy(entry);
        // "<mode> <type> <object>\t<path>"
        let Some((meta, path)) = entry.split_once('\t') else {
            return Err(format!("git ls-tree: unexpected entry {entry:?}"));
        };
        let object = meta.split(' ').nth(2).unwrap_or_default();
        files.insert(path.to_string(), object.to_string());
    }
    Ok(files)
}

/// Ask Nix which version it is — once per run, at startup.
///
/// This FAILS LOUDLY when `nix` is missing rather than falling back to a
/// placeholder: a key that quietly drops the version would hand yesterday's
/// `ok` to a different evaluator. The CI pins its Nix version for the same
/// reason.
pub fn nix_version() -> Result<String, String> {
    let out = std::process::Command::new("nix")
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run nix --version: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "nix --version failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
