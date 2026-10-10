//! One case, and then all of them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::build::{self, Outcome as Build};
use crate::cache::Cache;
use crate::cases::Case;
use crate::config::Config;
use crate::ledger::Ledger;
use crate::lever;
use crate::message;
use crate::parse;
use crate::tree::{self, Tree};
use crate::verdict::Verdict;

pub struct Ctx {
    pub repo: PathBuf,
    pub commit: String,
    pub scratch: PathBuf,
    pub cfg: Config,
    pub cache: Cache,
    pub no_cache: bool,
    /// Where every ruling is written the moment it is made. `None` = the
    /// run keeps no ledger (0.4.1 behaviour).
    pub ledger: Option<Ledger>,
    /// After this instant no new case is started; the one under way is
    /// finished. `None` = no budget.
    pub deadline: Option<Instant>,
    /// The baseline of each target, built when its first case comes up:
    /// `None` = green, `Some(reason)` = red, the target's cases are skipped.
    pub baselines: Mutex<BTreeMap<String, Option<String>>>,
    /// Every outcome so far, shared with the signal handler — an interrupted
    /// run reports what it ruled instead of dying without a word.
    pub live: Arc<Mutex<Vec<Outcome>>>,
}

impl Ctx {
    pub fn out_of_time(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// The targets whose baseline turned out red, with the reason.
    pub fn red_baselines(&self) -> Vec<(String, String)> {
        self.baselines
            .lock()
            .map(|b| {
                b.iter()
                    .filter_map(|(t, r)| r.clone().map(|r| (t.clone(), r)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Is this target's clean tree green? Built once, on first use, under
    /// the lock — two workers asking for the same target wait for ONE build
    /// instead of starting two.
    ///
    /// ON FIRST USE, NOT UP FRONT (0.5.0): a run with a time budget rules
    /// the oldest fifty cases of several hundred. Building the baseline of
    /// every target first would spend the budget on targets that are not
    /// even due tonight.
    fn baseline_red(&self, name: &str) -> Result<Option<String>, String> {
        let mut known = self
            .baselines
            .lock()
            .map_err(|_| "baseline lock poisoned".to_string())?;
        if let Some(r) = known.get(name) {
            return Ok(r.clone());
        }
        let red = baseline(self, &[name.to_string()])?
            .into_iter()
            .next()
            .map(|(_, reason)| reason);
        known.insert(name.to_string(), red.clone());
        Ok(red)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub case_id: String,
    pub verdict: Verdict,
    pub detail: String,
    pub from_cache: bool,
    /// Age of the cache entry that answered this case, if one did.
    pub cache_age_days: Option<u64>,
}

pub fn shard(cases: &[Case], shard: usize, of: usize) -> Vec<&Case> {
    if of <= 1 {
        return cases.iter().collect();
    }
    cases
        .iter()
        .enumerate()
        .filter(|(i, _)| i % of == shard)
        .map(|(_, c)| c)
        .collect()
}

/// Build the unsabotaged tree of each given target, and return those whose
/// baseline has no green — each with the reason. Called per target, when its
/// first case comes up (`Ctx::baseline_red`).
///
/// Without this, a check that is ALREADY red on a clean tree makes every
/// single case report "fell, with the expected message" — 525 ticks for
/// nothing. `no-latest-tags` was in exactly that state for months.
///
/// A red baseline blocks ITS target, not the run. Until 0.3.0 the first red
/// one stopped everything: in the homeserver acceptance (2026-09-20) OpenWrt
/// rebuilt its package feeds, the router baseline went red on a hash
/// mismatch, and 540 cases on ten healthy targets went unjudged with it.
/// Every target is still built once; an unknown target is a configuration
/// error and stays one.
pub fn baseline(ctx: &Ctx, targets: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut red = Vec::new();
    for name in targets {
        let target = ctx
            .cfg
            .target
            .get(name)
            .ok_or_else(|| format!("unknown target {name}"))?;
        let at = ctx.scratch.join(format!("baseline-{name}"));
        let tree = make_tree(ctx, &at)?;
        let outcome = build::run(&tree.path, &target.attr, &ctx.cfg.lotse.build_class);
        tree.remove()?;
        let reason = match outcome {
            Build::Green => continue,
            Build::Red(text) => first_errors(&text),
            Build::Network(_) => "network, no ruling".to_string(),
            Build::QueueTimeout => "queue timed out, no ruling".to_string(),
            Build::Failed(e) => e,
        };
        red.push((name.clone(), reason));
    }
    Ok(red)
}

/// Create a throwaway tree, cleaning up a leftover of the same name first.
fn make_tree(ctx: &Ctx, at: &std::path::Path) -> Result<Tree, String> {
    // After a signal, no new tree: the cleanup is removing the old ones.
    if crate::stop::requested() {
        return Err("interrupted".into());
    }
    std::fs::create_dir_all(&ctx.scratch)
        .map_err(|e| format!("cannot create {}: {e}", ctx.scratch.display()))?;
    match Tree::create(&ctx.repo, &ctx.commit, at) {
        Ok(t) => Ok(t),
        Err(first) => {
            // A run that died leaves both the directory and git's record of
            // it behind. Clear both and try once more, so a crashed run does
            // not poison every later one.
            let _ = std::fs::remove_dir_all(at);
            let _ = tree::prune(&ctx.repo);
            Tree::create(&ctx.repo, &ctx.commit, at).map_err(|second| {
                format!("cannot create worktree (first: {first}) (after prune: {second})")
            })
        }
    }
}

pub fn one(case: &Case, ctx: &Ctx, slot: usize) -> Outcome {
    let out = |verdict, detail: String| Outcome {
        case_id: case.id.clone(),
        verdict,
        detail,
        from_cache: false,
        cache_age_days: None,
    };

    let Some(target) = ctx.cfg.target.get(&case.target) else {
        return out(Verdict::NotRed, format!("unknown target {}", case.target));
    };

    // The cache is asked BEFORE the tree is made: a hit costs nothing at all.
    let key = if ctx.no_cache {
        None
    } else {
        match ctx
            .cache
            .key(case, &target.attr, &ctx.cfg.cache.definitions)
        {
            Ok(k) => Some(k),
            Err(e) => return out(Verdict::NotRed, format!("cache key: {e}")),
        }
    };
    if let Some(k) = &key
        && let Some(hit) = ctx.cache.get(k)
    {
        return Outcome {
            case_id: case.id.clone(),
            verdict: Verdict::Ok,
            detail: String::new(),
            from_cache: true,
            cache_age_days: Some(hit.age_days),
        };
    }

    let at = ctx.scratch.join(format!("slot-{slot}"));
    let tree = match make_tree(ctx, &at) {
        Ok(t) => t,
        Err(e) => return out(Verdict::NotRed, e),
    };

    let (verdict, detail) = judge(case, ctx, target.attr.as_str(), &tree);
    let _ = tree.remove();

    if verdict == Verdict::Ok
        && let Some(k) = &key
    {
        let _ = ctx.cache.put_ok(k);
    }
    out(verdict, detail)
}

fn judge(case: &Case, ctx: &Ctx, attr: &str, tree: &Tree) -> (Verdict, String) {
    for l in &case.levers {
        if let Err(e) = lever::apply(l, &tree.path) {
            return (Verdict::DeadLever, format!("the lever itself failed: {e}"));
        }
    }

    match tree.is_dirty() {
        Ok(false) => {
            return (
                Verdict::DeadLever,
                "the lever changed nothing — the case is dead, not the check".into(),
            );
        }
        Err(e) => return (Verdict::DeadLever, e),
        Ok(true) => {}
    }

    let changed = match tree.changed_files() {
        Ok(c) => c,
        Err(e) => return (Verdict::DeadLever, e),
    };
    // A syntax error found here is NOT a verdict yet — see `verdict_of`.
    let broken = match parse::check(&tree.path, &changed) {
        Ok(b) => b,
        // COULD NOT MEASURE is not a finding. If nix-instantiate cannot be
        // started, nothing was learned about this case — calling it
        // `broken-nix` would report all 525 cases as broken because one tool
        // is missing. `main` checks for the tools before the first case, so
        // this path means one vanished mid-run.
        Err(e) => return (Verdict::QueueTimeout, format!("CANNOT MEASURE: {e}")),
    };

    let build = build::run(&tree.path, attr, &ctx.cfg.lotse.build_class);
    verdict_of(case, build, broken)
}

/// From the build's outcome to the verdict. A function of its inputs alone,
/// so it can be tested against REAL output (`tests/antworten.rs`).
///
/// Two rules in here were learned from the homeserver acceptance, 2026-09-22:
///
/// **The expected message beats the network guess.** Case 174 lost its
/// ruling to lotse: the assertion it waits for QUOTES an nftables error
/// ("Could not resolve hostname"), lotse's retry pattern matched the quote,
/// retried four times and exited 201. The expected message stood in every
/// attempt. An assertion message in the output means evaluation got as far
/// as the assertion — no network failure produces the sentence a case is
/// waiting for. So it is a ruling. A case that demands GREEN has no message
/// to wait for; for it the network stays no ruling.
///
/// **A syntax error is a verdict only when the check did NOT fire.** Case 199
/// creates a `.nix` file on purpose that a text-scanning check reads with
/// `grep` and never parses. Judging `broken-nix` before the build called a
/// working check broken. Now the build runs: if it fails with the expected
/// message, the check fired and the broken file did not matter; if it fails
/// with a different one, the parser is the likelier author — `broken-nix`,
/// the palette case. Costs one build per broken case, of which the inventory
/// has one.
pub fn verdict_of(case: &Case, build: Build, broken: Option<String>) -> (Verdict, String) {
    let expected = |text: &str| message::matches(text, &case.expect, case.compare);
    match build {
        Build::Network(text) if !case.green && expected(&text) => (Verdict::Ok, String::new()),
        Build::Network(_) => (Verdict::Network, String::new()),
        Build::QueueTimeout => (Verdict::QueueTimeout, String::new()),
        Build::Failed(e) => (Verdict::QueueTimeout, e),
        // A case that demands GREEN reads the same two outcomes the other way
        // round. Reporting a fired check as `not-red` here would say the
        // reverse of what happened.
        Build::Green if case.green => (Verdict::Ok, String::new()),
        Build::Red(_) if case.green && broken.is_some() => {
            (Verdict::BrokenNix, broken.unwrap_or_default())
        }
        Build::Red(text) if case.green => (Verdict::FalseAlarm, first_errors(&text)),
        // Green although a sabotaged file does not parse: the target never
        // evaluated that file. What is left is the plain finding.
        Build::Green => (Verdict::NotRed, String::new()),
        Build::Red(text) if expected(&text) => (Verdict::Ok, String::new()),
        Build::Red(_) if broken.is_some() => (Verdict::BrokenNix, broken.unwrap_or_default()),
        Build::Red(text) => (Verdict::OtherMessage, first_errors(&text)),
    }
}

/// Dry run: everything except the build. Answers "does the lever still hit,
/// and does the file still parse" — the cheap half of the question, and the
/// half that rots silently.
pub fn dry(cases: &[&Case], ctx: &Ctx) -> Vec<Outcome> {
    let mut out = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        if crate::stop::requested() {
            break;
        }
        let at = ctx.scratch.join(format!("dry-{}", i % 8));
        let tree = match make_tree(ctx, &at) {
            Ok(t) => t,
            Err(e) => {
                out.push(Outcome {
                    case_id: case.id.clone(),
                    verdict: Verdict::NotRed,
                    detail: e,
                    from_cache: false,
                    cache_age_days: None,
                });
                continue;
            }
        };
        let (verdict, detail) = dry_judge(case, &tree);
        let _ = tree.remove();
        out.push(Outcome {
            case_id: case.id.clone(),
            verdict,
            detail,
            from_cache: false,
            cache_age_days: None,
        });
    }
    out
}

fn dry_judge(case: &Case, tree: &Tree) -> (Verdict, String) {
    for l in &case.levers {
        if let Err(e) = lever::apply(l, &tree.path) {
            return (Verdict::DeadLever, format!("the lever itself failed: {e}"));
        }
    }
    match tree.is_dirty() {
        Ok(false) => (
            Verdict::DeadLever,
            "the lever changed nothing — the case is dead, not the check".into(),
        ),
        Err(e) => (Verdict::DeadLever, e),
        Ok(true) => match tree.changed_files() {
            Err(e) => (Verdict::DeadLever, e),
            Ok(changed) => match parse::check(&tree.path, &changed) {
                // Without a build this is as far as the probe can see. The
                // full run may still call it `ok` — a check that only reads
                // the file as text fires anyway (`verdict_of`).
                Ok(Some(msg)) => (
                    Verdict::BrokenNix,
                    format!(
                        "{msg} (dry run: a check that reads the file as text may fire anyway — `tamper run` decides)"
                    ),
                ),
                // See `judge`: a tool that will not start is not a finding.
                Err(e) => (Verdict::QueueTimeout, format!("CANNOT MEASURE: {e}")),
                // The lever hits and the result parses. This probe says no
                // more than that; whether the check then goes red needs a build.
                Ok(None) => (Verdict::Ok, String::new()),
            },
        },
    }
}

/// The first three lines that look like an error — enough to recognise the
/// message, short enough to read in a list of 525.
fn first_errors(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("error") || l.contains("assertion")
        })
        .take(3)
        .map(|l| l.trim().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the cases on `jobs` workers, IN THE ORDER GIVEN. Each worker holds
/// exactly one tree at a time; lotse decides when a build actually starts.
///
/// A case whose target has a red baseline is skipped and appears in no
/// outcome — `Ctx::red_baselines` names the target. A case that is not
/// reached before the budget runs out is not in the outcomes either; with a
/// ledger it simply keeps its old entry and comes first next time.
pub fn all(cases: &[&Case], ctx: &Ctx, jobs: usize) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Outcome>> = Mutex::new(Vec::new());
    let started = Instant::now();

    std::thread::scope(|scope| {
        for slot in 0..jobs.max(1) {
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    if crate::stop::requested() || ctx.out_of_time() {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(case) = cases.get(i) else { break };
                    // An unknown target is left to `one`, which names it.
                    if ctx.cfg.target.contains_key(&case.target) {
                        match ctx.baseline_red(&case.target) {
                            Ok(None) => {}
                            Ok(Some(_)) => continue,
                            Err(e) => {
                                eprintln!("tamper: baseline of {}: {e}", case.target);
                                continue;
                            }
                        }
                    }
                    let outcome = one(case, ctx, slot);
                    record(ctx, &outcome, started);
                    if let Ok(mut r) = results.lock() {
                        r.push(outcome);
                    }
                }
            });
        }
    });

    let mut out = results.into_inner().unwrap_or_default();
    out.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    out
}

/// Everything that must not wait for the end of the run: the ledger entry,
/// the copy for the signal handler, and one line on stderr.
///
/// THE LINE IS THE POINT. Up to 0.4.1 a run printed nothing until its
/// report. On the homeserver's runner that meant three hours of silence and
/// then "The operation was canceled." — nobody could tell whether it had
/// ruled two cases or two hundred.
fn record(ctx: &Ctx, outcome: &Outcome, started: Instant) {
    if outcome.from_cache {
        return;
    }
    if let Some(ledger) = &ctx.ledger
        && let Err(e) = ledger.put(
            &outcome.case_id,
            outcome.verdict,
            &ctx.commit,
            &outcome.detail,
            now_secs(),
        )
    {
        eprintln!("tamper: {e}");
    }
    let done = match ctx.live.lock() {
        Ok(mut live) => {
            live.retain(|o| o.case_id != outcome.case_id);
            live.push(outcome.clone());
            live.len()
        }
        Err(_) => 0,
    };
    eprintln!(
        "  {:<14} case {}  ({done} done, {} s)",
        outcome.verdict.slug(),
        outcome.case_id,
        started.elapsed().as_secs()
    );
}

/// Cases without a ruling get one more go, after everything else is done —
/// a second attempt only costs where something really was missing. Two
/// passes, never a third: what dies twice on the network is a finding about
/// the network and belongs in the report, not in a loop.
pub fn all_with_retry(cases: &[&Case], ctx: &Ctx, jobs: usize) -> Vec<Outcome> {
    let mut out = all(cases, ctx, jobs);
    let again: Vec<&Case> = cases
        .iter()
        .filter(|c| {
            out.iter()
                .any(|o| o.case_id == c.id && !o.verdict.is_ruling())
        })
        .copied()
        .collect();
    // No second pass once the time is up: it would start new builds after
    // the budget, which is the one thing a budget promises not to do.
    if again.is_empty() || ctx.out_of_time() || crate::stop::requested() {
        return out;
    }
    let second = all(&again, ctx, jobs);
    out.retain(|o| !again.iter().any(|c| c.id == o.case_id));
    out.extend(second);
    out.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    out
}
