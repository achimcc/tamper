use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lexopt::prelude::*;

use tamper::cases;
use tamper::config::Config;
use tamper::verdict::Verdict;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tamper: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut parser = lexopt::Parser::from_env();
    let mut command: Option<String> = None;
    let mut config = PathBuf::from("tamper.toml");

    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Value(v) if command.is_none() => {
                command = Some(v.string().map_err(|e| e.to_string())?);
            }
            Long("config") => {
                config = PathBuf::from(parser.value().map_err(|e| e.to_string())?);
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
        Some("list") => list(&config),
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
        None => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

const USAGE: &str = "\
tamper — mutation-test your build-time assertions

  tamper list [--config tamper.toml]   the cases, their targets, their soundness
  tamper rules                         the seven verdicts, each explained

  --version, --help";

fn list(config: &Path) -> Result<ExitCode, String> {
    let cfg = Config::load(config)?;
    let root = config.parent().unwrap_or(Path::new("."));
    let cases = cases::load_dir(&root.join(&cfg.cases))?;
    let problems = cases::validate(&cases, &cfg);

    for case in &cases {
        println!("{:<6} {:<14} {}", case.id, case.target, case.name);
    }
    println!("\n{} cases", cases.len());
    for target in cfg.target.keys() {
        let n = cases.iter().filter(|c| &c.target == target).count();
        println!("  {target}: {n}");
    }

    if problems.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!("\n{} problem(s):", problems.len());
    for p in &problems {
        eprintln!("  {p}");
    }
    Ok(ExitCode::from(1))
}
