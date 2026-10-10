//! The ledger in a real run — the binary, a real repository, and stand-ins
//! for the three tools that do not exist in a build sandbox.
//!
//! The stand-in for `lotse` IS the check under test: it "builds" the tree it
//! is started in, and fails with the expected message exactly when `a.conf`
//! says `kaputt`. A case that sabotages another file therefore leaves the
//! build green — the plain finding `not-red`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tamper::ledger::Ledger;
use tamper::verdict::Verdict;

const DAY: u64 = 86_400;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-lrun-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let mut perm = std::fs::metadata(path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(path, perm).unwrap();
}

fn stub_bin() -> PathBuf {
    let bin = tempdir("bin");
    for tool in ["nix-instantiate", "nix"] {
        executable(&bin.join(tool), "#!/bin/sh\necho 'nix (Nix) 0.0-stub'\n");
    }
    // `lotse run --class=eval -- nix build --no-link <attr>`, in the tree.
    executable(
        &bin.join("lotse"),
        r#"#!/bin/sh
[ "$1" = "--version" ] && { echo "lotse 0.0-stub"; exit 0; }
grep -q langsam a.conf 2>/dev/null && sleep "${STUB_LANGSAM:-0}"
sleep "${STUB_SLEEP:-0}"
if grep -q kaputt a.conf 2>/dev/null; then
  echo "error: Failed assertions: ERWARTET" >&2
  exit 1
fi
exit 0
"#,
    );
    bin
}

/// `faelle`: id and the sed lever's file and expression.
fn fixture(faelle: &[(&str, &str, &str)]) -> PathBuf {
    let repo = tempdir("repo");
    std::fs::create_dir_all(repo.join("sabotage")).unwrap();
    std::fs::write(repo.join("a.conf"), "x\n").unwrap();
    std::fs::write(repo.join("b.conf"), "x\n").unwrap();
    std::fs::write(
        repo.join("tamper.toml"),
        r#"
cases = "sabotage"
[target.server]
attr = ".#nope"
[cache]
definitions = ["a.conf"]
[lotse]
run_class = "pruefungen"
build_class = "eval"
"#,
    )
    .unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    write_cases(&repo, faelle);
    repo
}

fn write_cases(repo: &Path, faelle: &[(&str, &str, &str)]) {
    let mut text = String::new();
    for (id, file, sed) in faelle {
        text.push_str(&format!(
            "[[case]]\nid = \"{id}\"\nname = \"{id}\"\ntarget = \"server\"\n\
             expect = \"ERWARTET\"\nwhy = \"x\"\n\
             levers = [ {{ file = \"{file}\", sed = \"{sed}\" }} ]\n\n"
        ));
    }
    std::fs::write(repo.join("sabotage/a.toml"), text).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "--quiet", "-m", "faelle"]);
}

struct Env {
    repo: PathBuf,
    book: PathBuf,
    runtime: PathBuf,
    path: String,
}

impl Env {
    fn new(faelle: &[(&str, &str, &str)]) -> Env {
        Env {
            repo: fixture(faelle),
            book: tempdir("book"),
            runtime: tempdir("runtime"),
            path: format!(
                "{}:{}",
                stub_bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tamper"));
        c.args(["run", "--config", "tamper.toml", "--no-cache", "-j", "1"])
            .arg("--ledger")
            .arg(&self.book)
            .args(args)
            .current_dir(&self.repo)
            .env("PATH", &self.path)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CACHE_HOME", self.runtime.join("cache"));
        c
    }

    fn run(&self, args: &[&str]) -> Run {
        Run::of(self.command(args).output().unwrap())
    }

    fn ledger(&self) -> Ledger {
        Ledger::open(&self.book).unwrap()
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn of(out: Output) -> Run {
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// The ids in the order the run ruled them, read off the progress lines.
    fn ruled(&self) -> Vec<String> {
        self.stderr
            .lines()
            .filter_map(|l| l.split(" case ").nth(1))
            .filter_map(|rest| rest.split_whitespace().next())
            .map(str::to_string)
            .collect()
    }

    fn all(&self) -> String {
        format!(
            "exit {}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

const TRIFFT: &str = "s/x/kaputt/";

#[test]
fn every_ruling_is_in_the_ledger_and_on_stderr_as_it_is_made() {
    let env = Env::new(&[
        ("a", "a.conf", TRIFFT),
        ("b", "a.conf", TRIFFT),
        // Sabotages a file the check does not read: the build stays green.
        ("c", "b.conf", TRIFFT),
    ]);
    let run = env.run(&["--oldest-first"]);
    assert_eq!(run.code, 1, "{}", run.all());
    assert_eq!(run.ruled(), ["a", "b", "c"], "{}", run.all());
    assert!(run.stdout.contains("2 of 3 cases ok"), "{}", run.all());
    assert!(
        run.stdout
            .contains("ledger: 3 case(s); 3 ruled in this run, 0 not reached; 0 never ruled"),
        "{}",
        run.all()
    );
    // What this run found is a finding, not ALSO a standing one.
    assert!(!run.stdout.contains("STANDING"), "{}", run.all());

    let book = env.ledger();
    assert_eq!(book.get("a").unwrap().verdict, Some(Verdict::Ok));
    assert_eq!(book.get("c").unwrap().verdict, Some(Verdict::NotRed));
    let head = git(&env.repo, &["rev-parse", "HEAD"]);
    assert_eq!(book.get("a").unwrap().commit, head.trim());
}

#[test]
fn a_run_out_of_time_goes_on_where_the_last_one_stopped() {
    let env = Env::new(&[
        ("a", "a.conf", TRIFFT),
        ("b", "a.conf", TRIFFT),
        ("c", "a.conf", TRIFFT),
        ("d", "a.conf", TRIFFT),
    ]);
    // Every build takes a second; the baseline is one of them.
    let first = Run::of(
        env.command(&["--oldest-first", "--budget", "2s"])
            .env("STUB_SLEEP", "1")
            .output()
            .unwrap(),
    );
    let done = first.ruled();
    assert!(
        !done.is_empty() && done.len() < 4,
        "the budget must cut the run short, not empty it: {}",
        first.all()
    );
    assert_eq!(first.code, 0, "{}", first.all());
    assert!(
        first.stdout.contains(&format!(
            "{} ruled in this run, {} not reached; {} never ruled",
            done.len(),
            4 - done.len(),
            4 - done.len()
        )),
        "{}",
        first.all()
    );

    // The second run has all the time it wants. It must START with what the
    // first one did not reach, and only then come back to the rest.
    let second = env.run(&["--oldest-first"]);
    let order = second.ruled();
    assert_eq!(order.len(), 4, "{}", second.all());
    let (fresh, again) = order.split_at(4 - done.len());
    assert!(
        fresh.iter().all(|id| !done.contains(id)),
        "first {done:?}, then {order:?}"
    );
    assert_eq!(again, done.as_slice(), "first {done:?}, then {order:?}");
    assert!(second.stdout.contains("0 never ruled"), "{}", second.all());
}

#[test]
fn a_finding_stands_until_the_case_is_ruled_green() {
    let env = Env::new(&[("a", "a.conf", TRIFFT), ("c", "b.conf", TRIFFT)]);
    assert_eq!(env.run(&["--oldest-first"]).code, 1);

    // A run that rules NOTHING must still be red: the finding has not gone
    // away because the run that made it is over.
    let idle = env.run(&["--oldest-first", "--budget", "0s"]);
    assert!(idle.ruled().is_empty(), "{}", idle.all());
    assert_eq!(idle.code, 1, "{}", idle.all());
    assert!(idle.stdout.contains("STANDING FINDING"), "{}", idle.all());
    assert!(
        idle.stdout.contains("not-red        case c"),
        "{}",
        idle.all()
    );

    // Repair the case. The standing finding goes FIRST, before the older ok.
    write_cases(
        &env.repo,
        &[("a", "a.conf", TRIFFT), ("c", "a.conf", TRIFFT)],
    );
    let fixed = env.run(&["--oldest-first"]);
    assert_eq!(fixed.ruled(), ["c", "a"], "{}", fixed.all());
    assert_eq!(fixed.code, 0, "{}", fixed.all());
    assert!(!fixed.stdout.contains("STANDING"), "{}", fixed.all());
}

#[test]
fn an_old_entry_is_exit_five_and_a_deleted_case_is_forgotten() {
    let env = Env::new(&[("a", "a.conf", TRIFFT), ("b", "a.conf", TRIFFT)]);
    let book = env.ledger();
    let head = git(&env.repo, &["rev-parse", "HEAD"]);
    book.put("a", Verdict::Ok, head.trim(), "", now() - 10 * DAY)
        .unwrap();
    book.put("b", Verdict::Ok, head.trim(), "", now() - DAY)
        .unwrap();
    // A case that no longer exists, with a finding: it must neither keep
    // the run red nor age into the alarm.
    book.put(
        "gibt-es-nicht-mehr",
        Verdict::NotRed,
        "x",
        "",
        now() - 99 * DAY,
    )
    .unwrap();

    let metrics = env.runtime.join("tamper.prom");
    let strict = env.run(&[
        "--budget",
        "0s",
        "--max-age",
        "7",
        "--metrics",
        metrics.to_str().unwrap(),
    ]);
    assert_eq!(strict.code, 5, "{}", strict.all());
    assert!(strict.stdout.contains("OVERDUE"), "{}", strict.all());
    assert!(
        strict
            .stdout
            .contains("oldest entry 10 day(s) old (case a)"),
        "{}",
        strict.all()
    );
    assert!(book.get("gibt-es-nicht-mehr").is_none());

    let text = std::fs::read_to_string(&metrics).unwrap();
    assert!(text.lines().any(|l| l == "tamper_cases 2"), "{text}");
    assert!(
        text.lines().any(|l| l == "tamper_standing_findings 0"),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l == "tamper_ruled_this_run 0"),
        "{text}"
    );

    assert_eq!(env.run(&["--budget", "0s", "--max-age", "30"]).code, 0);
}

#[test]
fn a_new_case_is_due_after_max_age_not_on_its_first_day() {
    let env = Env::new(&[("a", "a.conf", TRIFFT)]);
    // First sighting: never ruled, but not overdue.
    let first = env.run(&["--budget", "0s", "--max-age", "7"]);
    assert_eq!(first.code, 0, "{}", first.all());
    assert!(first.stdout.contains("1 never ruled"), "{}", first.all());

    // Backdate the sighting: now it has waited longer than allowed.
    let entry = env.book.join("a.entry");
    let text = std::fs::read_to_string(&entry).unwrap();
    assert!(text.starts_with("verdict never\n"), "{text}");
    std::fs::write(
        &entry,
        format!("verdict never\nstamp {}\ncommit \n", now() - 8 * DAY),
    )
    .unwrap();
    assert_eq!(env.run(&["--budget", "0s", "--max-age", "7"]).code, 5);
}

#[test]
fn without_the_ledger_switches_nothing_is_written() {
    let env = Env::new(&[("a", "a.conf", TRIFFT)]);
    let state = env.runtime.join("state");
    let out = Command::new(env!("CARGO_BIN_EXE_tamper"))
        .args(["run", "--config", "tamper.toml", "--no-cache", "-j", "1"])
        .current_dir(&env.repo)
        .env("PATH", &env.path)
        .env("XDG_RUNTIME_DIR", &env.runtime)
        .env("XDG_CACHE_HOME", env.runtime.join("cache"))
        .env("XDG_STATE_HOME", &state)
        .output()
        .unwrap();
    let run = Run::of(out);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(run.stdout.contains("1 of 1 cases ok"), "{}", run.all());
    assert!(!run.stdout.contains("ledger:"), "{}", run.all());
    assert!(!state.exists(), "a plain run must not start a ledger");
}

#[test]
fn an_interrupted_run_says_what_it_had_ruled() {
    // The CI job that hits its time limit: SIGTERM in the middle of a build.
    let env = Env::new(&[
        ("a", "a.conf", TRIFFT),
        ("b", "a.conf", "s/x/kaputt langsam/"),
    ]);
    let mut child = env
        .command(&["--oldest-first"])
        .env("STUB_LANGSAM", "30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait for the first ruling to be announced — then `b` is building.
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    let start = Instant::now();
    loop {
        line.clear();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "stderr closed");
        if line.contains(" case a ") {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "no ruling came");
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        Command::new("bash")
            .args(["-c", &format!("kill -TERM {}", child.id())])
            .status()
            .unwrap()
            .success()
    );
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(128 + 15), "{stdout}");
    assert!(stdout.contains("INTERRUPTED"), "{stdout}");
    assert!(stdout.contains("1 of 1 cases ok"), "{stdout}");

    // And the ruling outlives the run.
    let book = env.ledger();
    assert_eq!(book.get("a").unwrap().verdict, Some(Verdict::Ok));
    assert_eq!(book.get("b").unwrap().verdict, None);
}
