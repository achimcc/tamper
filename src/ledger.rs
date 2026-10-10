//! The ledger — when was each case last RULED, and how.
//!
//! WHY THE CACHE CANNOT ANSWER THAT. The cache key covers the check
//! definitions of the tested commit, so one commit to `checks/` turns every
//! key at once: the cache knows "this exact case on this exact tree was ok",
//! and after the next push it knows nothing. And the full run that was meant
//! to bound a stale `ok` (`--no-cache`, weekly) writes no cache at all.
//!
//! What that left open, measured on the homeserver (2026-10-09): the weekly
//! run had grown from 116 cases to 789, each a full evaluation; four shards
//! on one runner, each cut off by its three-hour limit after roughly a
//! quarter of its cases. Eight runs, none green — and since the report is
//! printed at the end, twelve hours of rulings every Sunday left NOTHING
//! behind. "A stale `ok` lives at most seven days" had never been true.
//!
//! So the ledger keeps one small file per case id, written the moment the
//! case is ruled, independent of the tree:
//!
//! ```text
//! verdict ok
//! stamp 1791500000
//! commit 8b1c3b00…
//! detail <first error lines, for a finding>
//! ```
//!
//! With it a run can take the cases whose ruling is OLDEST, stop when its
//! time is up, and lose nothing — the next run goes on where this one
//! stopped. And the question "can every check still go red?" gets an answer
//! with a number in it: the age of the oldest ruling.
//!
//! A case that has never been ruled has an entry too (`verdict never`,
//! stamped when it was first seen). That makes its age measurable from the
//! first run on, so a new case is not "overdue" the day it is written, and
//! an old ledger cannot hide one that never gets its turn.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::cases::Case;
use crate::verdict::Verdict;

pub struct Ledger {
    dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `None` = seen, never ruled.
    pub verdict: Option<Verdict>,
    /// Seconds since the epoch: the ruling, or the first sighting.
    pub stamp: u64,
    /// The commit that was ruled. Empty for a case never ruled.
    pub commit: String,
    pub detail: String,
}

impl Entry {
    pub fn age_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.stamp)
    }

    pub fn is_standing_finding(&self) -> bool {
        self.verdict.is_some_and(Verdict::is_finding)
    }
}

const SUFFIX: &str = ".entry";

impl Ledger {
    pub fn open(dir: &Path) -> Result<Ledger, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create ledger dir {}: {e}", dir.display()))?;
        Ok(Ledger {
            dir: dir.to_path_buf(),
        })
    }

    /// An entry that cannot be read is treated as absent: the case is then
    /// seen anew and comes first. Guessing a verdict out of a damaged file
    /// would be the one way a ledger can lie.
    pub fn get(&self, id: &str) -> Option<Entry> {
        let text = std::fs::read_to_string(self.path(id)).ok()?;
        parse(&text)
    }

    /// Record a ruling. `network` and `queue-timeout` are not rulings and
    /// leave the entry as it is — a night without network must not make a
    /// case look freshly judged, nor wipe the finding that stood before.
    pub fn put(
        &self,
        id: &str,
        verdict: Verdict,
        commit: &str,
        detail: &str,
        now: u64,
    ) -> Result<(), String> {
        if !verdict.is_ruling() {
            return Ok(());
        }
        self.write(
            id,
            &Entry {
                verdict: Some(verdict),
                stamp: now,
                commit: commit.to_string(),
                detail: detail.to_string(),
            },
        )
    }

    /// Give every case without an entry one that says "seen at `now`, never
    /// ruled". Returns how many were new.
    pub fn note_seen(&self, cases: &[&Case], now: u64) -> Result<usize, String> {
        let mut new = 0;
        for case in cases {
            if self.get(&case.id).is_none() {
                self.write(
                    &case.id,
                    &Entry {
                        verdict: None,
                        stamp: now,
                        commit: String::new(),
                        detail: String::new(),
                    },
                )?;
                new += 1;
            }
        }
        Ok(new)
    }

    /// Drop the entries of cases that no longer exist. Without this a
    /// deleted case with a standing finding would keep the run red forever,
    /// and a deleted one that was never ruled would age into the alarm.
    pub fn prune(&self, known: &BTreeSet<String>) -> Result<usize, String> {
        let names: BTreeSet<String> = known.iter().map(|id| file_name(id)).collect();
        let mut gone = 0;
        let read = std::fs::read_dir(&self.dir)
            .map_err(|e| format!("cannot read ledger dir {}: {e}", self.dir.display()))?;
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(SUFFIX) && !names.contains(&name) {
                std::fs::remove_file(entry.path())
                    .map_err(|e| format!("cannot remove ledger entry {name}: {e}"))?;
                gone += 1;
            }
        }
        Ok(gone)
    }

    /// Written next to its final name and renamed into place: a run that is
    /// killed mid-write leaves the old entry, never half of a new one.
    fn write(&self, id: &str, entry: &Entry) -> Result<(), String> {
        let path = self.path(id);
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, render(entry))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("cannot write ledger entry for case {id}: {e}"))
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(file_name(id))
    }
}

/// A case id is free text; a file name is not. Letters, digits, `-` and `_`
/// stay readable, every other byte becomes `%XX` — so `a/b` cannot leave the
/// directory, and two different ids can never share a file.
fn file_name(id: &str) -> String {
    let mut out = String::new();
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str(SUFFIX);
    out
}

fn render(entry: &Entry) -> String {
    let mut out = format!(
        "verdict {}\nstamp {}\ncommit {}\n",
        entry.verdict.map_or("never", Verdict::slug),
        entry.stamp,
        entry.commit
    );
    for line in entry.detail.lines() {
        out.push_str(&format!("detail {line}\n"));
    }
    out
}

fn parse(text: &str) -> Option<Entry> {
    let mut verdict: Option<Option<Verdict>> = None;
    let mut stamp = None;
    let mut commit = String::new();
    let mut detail: Vec<&str> = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "verdict" if value == "never" => verdict = Some(None),
            "verdict" => verdict = Some(Some(Verdict::from_slug(value)?)),
            "stamp" => stamp = value.parse().ok(),
            "commit" => commit = value.to_string(),
            "detail" => detail.push(value),
            _ => {}
        }
    }
    Some(Entry {
        verdict: verdict?,
        stamp: stamp?,
        commit,
        detail: detail.join("\n"),
    })
}

/// The order of a run that may not get through everything: what has never
/// been ruled first, then what stands as a finding (so a repaired case turns
/// green the next night instead of in two weeks), then `ok` from the oldest
/// to the youngest. The id breaks ties, so two runs over the same ledger
/// agree on the order.
pub fn oldest_first<'a>(cases: &[&'a Case], ledger: &Ledger) -> Vec<&'a Case> {
    let mut keyed: Vec<(u8, u64, &'a Case)> = cases
        .iter()
        .map(|case| match ledger.get(&case.id) {
            None => (0, 0, *case),
            Some(e) if e.verdict.is_none() => (0, e.stamp, *case),
            Some(e) if e.is_standing_finding() => (1, e.stamp, *case),
            Some(e) => (2, e.stamp, *case),
        })
        .collect();
    keyed.sort_by(|a, b| (a.0, a.1, &a.2.id).cmp(&(b.0, b.1, &b.2.id)));
    keyed.into_iter().map(|(_, _, case)| case).collect()
}

/// What the ledger says about a selection of cases, at one moment.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub cases: usize,
    /// Ids of the cases never ruled.
    pub never_ruled: Vec<String>,
    /// Findings the ledger holds: id, verdict, detail, age in seconds, commit.
    pub findings: Vec<Standing>,
    /// Age of the oldest entry — a ruling, or the first sighting of a case
    /// never ruled. `None` only for an empty selection.
    pub oldest_secs: Option<u64>,
    pub oldest_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Standing {
    pub id: String,
    pub verdict: Verdict,
    pub detail: String,
    pub age_secs: u64,
    pub commit: String,
}

pub fn state(cases: &[&Case], ledger: &Ledger, now: u64) -> State {
    let mut s = State {
        cases: cases.len(),
        ..State::default()
    };
    for case in cases {
        // No entry at all only happens when `note_seen` did not run for
        // this case; it counts as never ruled, with no age yet.
        let (age, entry) = match ledger.get(&case.id) {
            Some(e) => (e.age_secs(now), Some(e)),
            None => (0, None),
        };
        match &entry {
            None => s.never_ruled.push(case.id.clone()),
            Some(e) => match e.verdict {
                None => s.never_ruled.push(case.id.clone()),
                Some(v) if v.is_finding() => s.findings.push(Standing {
                    id: case.id.clone(),
                    verdict: v,
                    detail: e.detail.clone(),
                    age_secs: age,
                    commit: e.commit.clone(),
                }),
                Some(_) => {}
            },
        }
        if s.oldest_secs.is_none_or(|o| age > o) {
            s.oldest_secs = Some(age);
            s.oldest_id = Some(case.id.clone());
        }
    }
    s
}

/// The state as a Prometheus text file, for a textfile collector.
pub fn metrics(state: &State, ruled_this_run: usize, now: u64) -> String {
    format!(
        "# HELP tamper_cases Sabotage cases in the selection of this run.\n\
         # TYPE tamper_cases gauge\n\
         tamper_cases {}\n\
         # HELP tamper_never_ruled Cases that have never been ruled.\n\
         # TYPE tamper_never_ruled gauge\n\
         tamper_never_ruled {}\n\
         # HELP tamper_standing_findings Cases whose last ruling is a finding.\n\
         # TYPE tamper_standing_findings gauge\n\
         tamper_standing_findings {}\n\
         # HELP tamper_oldest_ruling_seconds Age of the oldest ruling, or of the first sighting of a case never ruled.\n\
         # TYPE tamper_oldest_ruling_seconds gauge\n\
         tamper_oldest_ruling_seconds {}\n\
         # HELP tamper_ruled_this_run Cases the last run ruled.\n\
         # TYPE tamper_ruled_this_run gauge\n\
         tamper_ruled_this_run {}\n\
         # HELP tamper_last_run_timestamp_seconds When the last run wrote this file.\n\
         # TYPE tamper_last_run_timestamp_seconds gauge\n\
         tamper_last_run_timestamp_seconds {}\n",
        state.cases,
        state.never_ruled.len(),
        state.findings.len(),
        state.oldest_secs.unwrap_or(0),
        ruled_this_run,
        now
    )
}

/// `170m`, `3h`, `90s`. The unit is required: a bare `170` would be read as
/// seconds by one person and as minutes by the next.
pub fn parse_duration(text: &str) -> Result<std::time::Duration, String> {
    let bad = || format!("`{text}` is not a duration — write it like 170m, 3h or 90s");
    let split = text.len().checked_sub(1).ok_or_else(bad)?;
    if !text.is_char_boundary(split) {
        return Err(bad());
    }
    let (number, unit) = text.split_at(split);
    let n: u64 = number.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60).ok_or_else(bad)?,
        "h" => n.checked_mul(3600).ok_or_else(bad)?,
        _ => return Err(bad()),
    };
    Ok(std::time::Duration::from_secs(secs))
}
