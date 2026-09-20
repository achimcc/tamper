use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use lexopt::prelude::*;

use tamper::cache::Cache;
use tamper::cases::{self, Case};
use tamper::config::Config;
use tamper::report::Summary;
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
  --no-cache        ignore the result cache — the mode of the weekly CI run
  --shard I --of N  run only every Nth case, starting at I
  --commit REF      the commit to test (default: HEAD)
  --version, --help

EXIT CODES
  0  every case ok
  1  at least one finding
  2  tamper could not run
  3  the baseline is red — no ruling about any case
  4  no finding, but cases without a ruling (network, queue)";

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
    // for a worktree.
    tree::prune(&repo)?;

    let chosen: Vec<&Case> = run::shard(&cases, o.shard, o.of.max(1))
        .into_iter()
        .filter(|c| o.targets.is_empty() || o.targets.contains(&c.target))
        .filter(|c| o.ids.is_empty() || o.ids.contains(&c.id))
        .collect();
    if chosen.is_empty() {
        return Err("no case matches the selection".into());
    }

    let commit = match &o.commit {
        Some(c) => c.clone(),
        None => head(&repo)?,
    };
    let ctx = Ctx {
        repo: repo.clone(),
        commit,
        scratch: scratch_dir(&repo)?,
        // Measured once for the whole run, not once per case.
        cache: Cache::open(&cache_dir(&repo)?, &repo, &tamper::cache::nix_version()?)?,
        cfg,
        no_cache: o.no_cache || dry,
    };

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
        let mut targets: Vec<String> = chosen.iter().map(|c| c.target.clone()).collect();
        targets.sort();
        targets.dedup();
        if let Err(e) = run::baseline(&ctx, &targets) {
            eprintln!("tamper: {e}");
            return Ok(ExitCode::from(3));
        }
        run::all_with_retry(&chosen, &ctx, o.jobs)
    };

    let summary = Summary::of(&outcomes);
    print!("{}", summary.render());
    Ok(ExitCode::from(summary.exit_code()))
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
fn scratch_dir(repo: &Path) -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("tamper").join(slug(repo));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn cache_dir(repo: &Path) -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither XDG_CACHE_HOME nor HOME is set")?;
    Ok(base.join("tamper").join(slug(repo)))
}

/// A readable, collision-free name for a repository path.
fn slug(repo: &Path) -> String {
    repo.to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}
