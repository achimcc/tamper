//! One case, and then all of them.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::build::{self, Outcome as Build};
use crate::cache::Cache;
use crate::cases::Case;
use crate::config::Config;
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

/// Build the unsabotaged tree once per target.
///
/// Without this, a check that is ALREADY red on a clean tree makes every
/// single case report "fell, with the expected message" — 525 ticks for
/// nothing. `no-latest-tags` was in exactly that state for months.
pub fn baseline(ctx: &Ctx, targets: &[String]) -> Result<(), String> {
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
        match outcome {
            Build::Green => {}
            Build::Red(text) => {
                return Err(format!(
                    "baseline for target `{name}` is RED — no case can say anything \
                     while the clean tree does not build:\n{}",
                    first_errors(&text)
                ));
            }
            Build::Network => return Err(format!("baseline for `{name}`: network, no ruling")),
            Build::QueueTimeout => {
                return Err(format!("baseline for `{name}`: queue timed out, no ruling"));
            }
            Build::Failed(e) => return Err(format!("baseline for `{name}`: {e}")),
        }
    }
    Ok(())
}

/// Create a throwaway tree, cleaning up a leftover of the same name first.
fn make_tree(ctx: &Ctx, at: &std::path::Path) -> Result<Tree, String> {
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
        match ctx.cache.key(case, &ctx.cfg.cache.definitions) {
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
    match parse::check(&tree.path, &changed) {
        Ok(Some(msg)) => return (Verdict::BrokenNix, msg),
        Err(e) => return (Verdict::BrokenNix, e),
        Ok(None) => {}
    }

    match build::run(&tree.path, attr, &ctx.cfg.lotse.build_class) {
        Build::Green => (Verdict::NotRed, String::new()),
        Build::Network => (Verdict::Network, String::new()),
        Build::QueueTimeout => (Verdict::QueueTimeout, String::new()),
        Build::Failed(e) => (Verdict::QueueTimeout, e),
        Build::Red(text) => {
            if message::matches(&text, &case.expect, case.compare) {
                (Verdict::Ok, String::new())
            } else {
                (Verdict::OtherMessage, first_errors(&text))
            }
        }
    }
}

/// Dry run: everything except the build. Answers "does the lever still hit,
/// and does the file still parse" — the cheap half of the question, and the
/// half that rots silently.
pub fn dry(cases: &[&Case], ctx: &Ctx) -> Vec<Outcome> {
    let mut out = Vec::new();
    for (i, case) in cases.iter().enumerate() {
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
                Ok(Some(msg)) => (Verdict::BrokenNix, msg),
                Err(e) => (Verdict::BrokenNix, e),
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
        .map(|l| format!("        {}", l.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the cases on `jobs` workers. Each worker holds exactly one tree at a
/// time; lotse decides when a build actually starts.
pub fn all(cases: &[&Case], ctx: &Ctx, jobs: usize) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Outcome>> = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for slot in 0..jobs.max(1) {
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(case) = cases.get(i) else { break };
                    let outcome = one(case, ctx, slot);
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
    if again.is_empty() {
        return out;
    }
    let second = all(&again, ctx, jobs);
    out.retain(|o| !again.iter().any(|c| c.id == o.case_id));
    out.extend(second);
    out.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    out
}
