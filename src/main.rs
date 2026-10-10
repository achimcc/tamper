use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lexopt::prelude::*;

use tamper::cache::Cache;
use tamper::cases::{self, Case};
use tamper::config::Config;
use tamper::ledger::{self, Ledger};
use tamper::report::{Blocked, Book, Summary};
use tamper::run::{self, Ctx};
use tamper::tree;
use tamper::verdict::Verdict;

const USAGE: &str = "\
tamper — mutation-test your build-time assertions

  tamper run [OPTIONS]    break each case's file, build, judge the message
  tamper dry [OPTIONS]    does each lever still hit? (no build, seconds)
  tamper list [OPTIONS]   the cases, their targets, their soundness
  tamper import FILE      one-shot: turn the shell driver into TOML
  tamper rules            the eight verdicts, each explained

OPTIONS
  --config FILE     tamper.toml (default: ./tamper.toml)
  --target NAME     only cases for this target (repeatable)
  --case ID         only this case (repeatable)
  -j, --jobs N      workers offered; lotse decides when a build starts (default 4)
  --no-cache        ignore the result cache — the mode of the periodic CI run
  --shard I --of N  run only every Nth case, starting at I
  --commit REF      the commit to test (default: HEAD)
  --version, --help

THE LEDGER — for a run that cannot get through every case in one go
  --oldest-first    rule what was never ruled first, then standing findings,
                    then `ok` from the oldest ruling to the youngest
  --budget TIME     start no new case after TIME (170m, 3h, 90s); the one
                    under way is finished, the rest keeps its place in line
  --max-age DAYS    exit 5 if the oldest entry is older than DAYS
  --metrics FILE    write the ledger's state as a Prometheus text file
  --ledger DIR      where the ledger lives
                    (default: $XDG_STATE_HOME/tamper/<repository>)
  Any of these but --budget makes the run keep a ledger: every ruling is
  written the moment it is made, and a finding stands until the case is
  ruled again.

EXIT CODES
  0  every case ok
  1  at least one finding
  2  tamper could not run
  3  a target's baseline is red — its cases were not run (the others were)
  4  no finding, but cases without a ruling (network, queue)
  5  no finding, but the ledger's oldest entry is older than --max-age";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tamper: {e}");
            ExitCode::from(2)
        }
    }
}

#[derive(Default)]
struct Opts {
    config: Option<PathBuf>,
    targets: Vec<String>,
    ids: Vec<String>,
    jobs: usize,
    no_cache: bool,
    shard: usize,
    of: usize,
    commit: Option<String>,
    rest: Vec<String>,
    oldest_first: bool,
    budget: Option<Duration>,
    max_age_days: Option<u64>,
    metrics: Option<PathBuf>,
    ledger: Option<PathBuf>,
}

impl Opts {
    /// `--budget` alone only cuts a run short; everything else needs the
    /// ledger to mean anything.
    fn keeps_ledger(&self) -> bool {
        self.oldest_first
            || self.max_age_days.is_some()
            || self.metrics.is_some()
            || self.ledger.is_some()
    }
}

fn run() -> Result<ExitCode, String> {
    let mut parser = lexopt::Parser::from_env();
    let mut command: Option<String> = None;
    let mut o = Opts {
        jobs: 4,
        of: 1,
        ..Default::default()
    };

    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Value(v) if command.is_none() => {
                command = Some(v.string().map_err(|e| e.to_string())?);
            }
            Value(v) => o.rest.push(v.string().map_err(str_err)?),
            Long("config") => o.config = Some(PathBuf::from(parser.value().map_err(str_err)?)),
            Long("target") => o
                .targets
                .push(parser.value().map_err(str_err)?.string().map_err(str_err)?),
            Long("case") => o
                .ids
                .push(parser.value().map_err(str_err)?.string().map_err(str_err)?),
            Long("jobs") | Short('j') => {
                o.jobs = parser.value().map_err(str_err)?.parse().map_err(str_err)?
            }
            Long("no-cache") => o.no_cache = true,
            Long("shard") => o.shard = parser.value().map_err(str_err)?.parse().map_err(str_err)?,
            Long("of") => o.of = parser.value().map_err(str_err)?.parse().map_err(str_err)?,
            Long("commit") => {
                o.commit = Some(parser.value().map_err(str_err)?.string().map_err(str_err)?)
            }
            Long("oldest-first") => o.oldest_first = true,
            Long("budget") => {
                let text = parser.value().map_err(str_err)?.string().map_err(str_err)?;
                o.budget = Some(ledger::parse_duration(&text)?);
            }
            Long("max-age") => {
                o.max_age_days = Some(parser.value().map_err(str_err)?.parse().map_err(str_err)?)
            }
            Long("metrics") => o.metrics = Some(PathBuf::from(parser.value().map_err(str_err)?)),
            Long("ledger") => o.ledger = Some(PathBuf::from(parser.value().map_err(str_err)?)),
            Long("help") | Short('h') => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            Long("version") | Short('V') => {
                println!("tamper {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(other.unexpected().to_string()),
        }
    }

    match command.as_deref() {
        Some("rules") => {
            for v in Verdict::ALL {
                println!("{:<14} {}", v.slug(), v.explain());
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("list") => list(&o),
        Some("run") => execute(&o, false),
        Some("dry") => execute(&o, true),
        Some("import") => {
            let file = o
                .rest
                .first()
                .ok_or("import needs the path of the shell driver")?;
            import(Path::new(file))
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
        None => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn str_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Load the config and the cases, and refuse to go on if the set is unsound.
/// A run against cases that do not validate would measure the cases, not the
/// checks.
fn load(o: &Opts) -> Result<(PathBuf, Config, Vec<Case>), String> {
    let config = o
        .config
        .clone()
        .unwrap_or_else(|| PathBuf::from("tamper.toml"));
    let cfg = Config::load(&config)?;
    let root = config
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let cases = cases::load_dir(&root.join(&cfg.cases))?;

    let problems = cases::validate(&cases, &cfg);
    if !problems.is_empty() {
        let mut msg = format!("{} problem(s) in the cases:", problems.len());
        for p in &problems {
            msg.push_str(&format!("\n  {p}"));
        }
        return Err(msg);
    }
    Ok((root, cfg, cases))
}

fn list(o: &Opts) -> Result<ExitCode, String> {
    let (_, cfg, cases) = load(o)?;
    for case in &cases {
        println!("{:<6} {:<14} {}", case.id, case.target, case.name);
    }
    println!("\n{} cases", cases.len());
    for target in cfg.target.keys() {
        let n = cases.iter().filter(|c| &c.target == target).count();
        println!("  {target}: {n}");
    }
    Ok(ExitCode::SUCCESS)
}

/// One-shot migration: TOML to stdout, every leftover to stderr by name.
/// Exit 1 when anything is left over, so a redirect cannot look finished.
fn import(file: &Path) -> Result<ExitCode, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let got = tamper::import::parse_script(&text);
    print!("{}", tamper::import::to_toml(&got.cases));

    eprintln!(
        "\n{} of {} calls became cases; {} of them got an id made up from their name \
         (their comment block carried no number).\n{} leftover(s):",
        got.cases.len(),
        got.seen_calls,
        got.synthetic_ids,
        got.leftovers.len()
    );
    for l in &got.leftovers {
        eprintln!("  {}:{}  {}", file.display(), l.line, l.reason);
        eprintln!("      {}", l.snippet);
    }
    if got.leftovers.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

/// Every external tool a run needs, asked for once before the first case.
///
/// Without this, a missing tool is discovered 525 times over and reads like
/// a finding about the cases. `nix-instantiate` was exactly that: absent, it
/// turned every case into `broken-nix`.
fn preflight(dry: bool) -> Result<(), String> {
    let needed: &[&str] = if dry {
        &["git", "sed", "perl", "nix-instantiate"]
    } else {
        &["git", "sed", "perl", "nix-instantiate", "nix", "lotse"]
    };
    let missing: Vec<&str> = needed
        .iter()
        .copied()
        .filter(|tool| {
            Command::new(tool)
                .arg("--version")
                .output()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "these tools are missing, and without them a run says nothing: {}",
            missing.join(", ")
        ))
    }
}

fn execute(o: &Opts, dry: bool) -> Result<ExitCode, String> {
    preflight(dry)?;
    let (root, cfg, cases) = load(o)?;
    let repo = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", root.display()))?;

    // Clear what a run that died left behind, before anything else asks git
    // for a worktree: first git's records of vanished trees, then the trees
    // of runs whose process is gone — their directories still exist, so
    // `prune` alone keeps them (audit 3, CD-8).
    tree::prune(&repo)?;
    let base = scratch_base(&repo);
    // A leftover that will not go is worth a line, not the whole run.
    if let Err(e) = tree::sweep_dead_runs(&repo, &base) {
        eprintln!("tamper: could not clear what a dead run left behind: {e}");
    }

    let mut chosen: Vec<&Case> = run::shard(&cases, o.shard, o.of.max(1))
        .into_iter()
        .filter(|c| o.targets.is_empty() || o.targets.contains(&c.target))
        .filter(|c| o.ids.is_empty() || o.ids.contains(&c.id))
        .collect();
    if chosen.is_empty() {
        return Err("no case matches the selection".into());
    }

    // The ledger, before the first case: forget the cases that are gone,
    // give the new ones their first entry, and only then decide the order —
    // so a case written today is in tonight's front row.
    let book = if !dry && o.keeps_ledger() {
        let dir = match &o.ledger {
            Some(dir) => dir.clone(),
            None => state_dir(&repo)?,
        };
        let book = Ledger::open(&dir)?;
        book.prune(&cases.iter().map(|c| c.id.clone()).collect())?;
        book.note_seen(&chosen, now_secs())?;
        if o.oldest_first {
            chosen = ledger::oldest_first(&chosen, &book);
        }
        Some(book)
    } else {
        None
    };

    let commit = match &o.commit {
        Some(c) => c.clone(),
        None => head(&repo)?,
    };
    // The key reads the SAME commit that gets built — never the working tree.
    // The Nix version is measured once for the whole run, not once per case.
    let cache = Cache::open(
        &cache_dir(&repo)?,
        &repo,
        &commit,
        &tamper::cache::nix_version()?,
    )?;
    let scratch = scratch_dir(&base)?;
    let live: Arc<Mutex<Vec<run::Outcome>>> = Arc::new(Mutex::new(Vec::new()));
    // From here on a tree may exist, so an interruption must remove it —
    // and say what the run had ruled until then. A CI job that hits its
    // time limit ends exactly here.
    {
        let (repo, scratch, live) = (repo.clone(), scratch.clone(), live.clone());
        tamper::stop::install(move || {
            if let Err(e) = tree::remove_all_under(&repo, &scratch) {
                eprintln!("tamper: interrupted, and the cleanup failed: {e}");
            }
            if let Ok(done) = live.lock()
                && !done.is_empty()
            {
                println!(
                    "INTERRUPTED — this is what the run had ruled until then, not a result \
                     for the whole selection:"
                );
                print!("{}", Summary::of(&done).render());
            }
        });
    }
    let ctx = Ctx {
        repo: repo.clone(),
        commit,
        scratch,
        cache,
        cfg,
        no_cache: o.no_cache || dry,
        ledger: book,
        deadline: o.budget.map(|b| Instant::now() + b),
        baselines: Mutex::new(BTreeMap::new()),
        live,
    };

    let mut blocked: Vec<Blocked> = Vec::new();
    let outcomes = if dry {
        let out = run::dry(&chosen, &ctx);
        let hit = out.iter().filter(|o| o.verdict == Verdict::Ok).count();
        println!(
            "{hit} of {} levers hit. This says nothing about whether the check then goes\n\
             red — that needs `tamper run`.",
            out.len()
        );
        out
    } else {
        // Each target's baseline is built when its first case comes up
        // (`Ctx::baseline_red`); what turned out red is collected afterwards.
        let out = run::all_with_retry(&chosen, &ctx, o.jobs);
        blocked = ctx
            .red_baselines()
            .into_iter()
            .map(|(target, reason)| Blocked {
                cases: chosen.iter().filter(|c| c.target == target).count(),
                target,
                reason,
            })
            .collect();
        out
    };

    // Das eigene Laufverzeichnis wieder abraeumen: Die Baeume darin sind
    // einzeln entfernt, was bleibt, ist die leere Huelle.
    let _ = std::fs::remove_dir_all(&ctx.scratch);

    let mut summary = Summary::of(&outcomes);
    if let Some(book) = &ctx.ledger {
        let now = now_secs();
        let state = ledger::state(&chosen, book, now);
        let ruled_this_run = outcomes
            .iter()
            .filter(|o| o.verdict.is_ruling() && !o.from_cache)
            .count();
        if let Some(file) = &o.metrics {
            write_atomically(file, &ledger::metrics(&state, ruled_this_run, now))?;
        }
        let in_blocked: usize = blocked.iter().map(|b| b.cases).sum();
        summary = summary.with_book(Book {
            // A finding this run made is already in the list above; what
            // STANDS is what the ledger holds for the cases it did not rule.
            standing: state
                .findings
                .iter()
                .filter(|s| !outcomes.iter().any(|o| o.case_id == s.id))
                .cloned()
                .collect(),
            ruled_this_run,
            not_reached: chosen
                .len()
                .saturating_sub(outcomes.len())
                .saturating_sub(in_blocked),
            max_age_days: o.max_age_days,
            state,
        });
    }
    let summary = summary.with_blocked(blocked);
    print!("{}", summary.render());
    Ok(ExitCode::from(summary.exit_code()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A collector may read the file at any moment; half a file would be a
/// parse error there, not just a wrong number.
fn write_atomically(file: &Path, text: &str) -> Result<(), String> {
    let tmp = file.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, file))
        .map_err(|e| format!("cannot write {}: {e}", file.display()))
}

fn head(repo: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cannot read HEAD of {}: {}",
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Throwaway worktrees live outside the repository, so a run cannot be
/// mistaken for work in progress.
///
/// PER RUN, NOT PER REPOSITORY. The slot paths used to depend on the repo
/// alone, so two runs at once shared `slot-0`…`slot-n` and removed each
/// other's trees. The result was a FALSE `dead-lever` — a finding pointing
/// at the case, for something the case had nothing to do with. On a
/// workstation with a dozen sessions that is not an edge case.
fn scratch_dir(base: &Path) -> Result<PathBuf, String> {
    let dir = base.join(format!("run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Where every run of this repository keeps its `run-<pid>` directory.
fn scratch_base(repo: &Path) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("tamper")
        .join(slug(repo))
}

fn cache_dir(repo: &Path) -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither XDG_CACHE_HOME nor HOME is set")?;
    Ok(base.join("tamper").join(slug(repo)))
}

/// The ledger is STATE, not cache: losing it is losing the record of what
/// was ruled when, and every case would look new.
fn state_dir(repo: &Path) -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .ok_or("neither XDG_STATE_HOME nor HOME is set — pass --ledger DIR")?;
    Ok(base.join("tamper").join(slug(repo)))
}

/// A readable, collision-free name for a repository path.
fn slug(repo: &Path) -> String {
    repo.to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}
