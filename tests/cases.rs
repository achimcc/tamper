use std::io::Write;

use tamper::cases::{self, Compare, Lever};
use tamper::config::Config;

fn write(dir: &std::path::Path, name: &str, body: &str) {
    let mut f = std::fs::File::create(dir.join(name)).unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

fn config() -> Config {
    Config::parse(
        r#"
cases = "sabotage"
[target.server]
attr = ".#nixosConfigurations.server.config.system.build.toplevel"
[cache]
definitions = ["checks.nix"]
[lotse]
run_class = "pruefungen"
build_class = "eval"
"#,
    )
    .unwrap()
}

#[test]
fn reads_a_case_with_a_sed_lever() {
    let dir = tempdir();
    write(
        &dir,
        "guests.toml",
        r#"
[[case]]
id = "9c"
name = "Bind mit :idmap ueber einem eigenen Mount"
target = "server"
expect = "ueber einem eigenen Mount"
why = "Befund 2026-09-02, stand neun Stunden im Repo."
levers = [ { file = "lib/gaeste.nix", sed = "s|a|b|" } ]
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].id, "9c");
    // regex is the default, because 30 of 522 patterns use metacharacters
    // on purpose (a `.` standing in for a backtick).
    assert_eq!(cases[0].compare, Compare::Regex);
    assert!(matches!(cases[0].levers[0], Lever::Sed { .. }));
    assert_eq!(
        cases[0].levers[0].files(),
        vec!["lib/gaeste.nix".to_string()]
    );
}

#[test]
fn a_script_lever_must_declare_its_files() {
    // The cache key is computed BEFORE the lever runs, so a script lever
    // cannot be asked what it will touch — it has to say so.
    let dir = tempdir();
    write(
        &dir,
        "x.toml",
        r#"
[[case]]
id = "39"
name = "inventar.env aus dem Index"
target = "server"
expect = "ist veraltet"
why = "x"
levers = [ { script = "git rm --cached -q scripts/inventar.env" } ]
"#,
    );
    let err = cases::load_dir(&dir).unwrap_err();
    assert!(
        err.contains("files"),
        "error should name the missing key: {err}"
    );
}

#[test]
fn duplicate_ids_are_an_error() {
    // 45 case numbers were handed out twice by parallel sessions (2026-09-13).
    let dir = tempdir();
    write(&dir, "a.toml", &one_case("51"));
    write(&dir, "b.toml", &one_case("51"));
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(problems.iter().any(|p| p.contains("51")), "{problems:?}");
}

#[test]
fn an_unknown_target_is_an_error() {
    let dir = tempdir();
    write(&dir, "a.toml", &one_case_for("7", "rooter"));
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(
        problems.iter().any(|p| p.contains("rooter")),
        "{problems:?}"
    );
}

#[test]
fn a_pattern_that_does_not_compile_is_an_error() {
    // `fehlt {EntryData}` is literal in grep's BRE and may not be here.
    // Whatever this engine does with it, we must find out at load time and
    // not in the middle of a two-hour run.
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
expect = "a("
why = "x"
levers = [ { file = "f.nix", sed = "s|a|b|" } ]
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(
        problems.iter().any(|p| p.contains("expect")),
        "{problems:?}"
    );
}

#[test]
fn a_case_without_a_lever_is_an_error() {
    // Worse than a dead lever: the case builds an unchanged tree and reports
    // green — it proves the opposite of what it claims.
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
expect = "x"
why = "x"
levers = []
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(
        problems.iter().any(|p| p.contains("no lever")),
        "{problems:?}"
    );
}

#[test]
fn a_case_without_a_why_is_an_error() {
    // The 6000 lines being replaced are mostly reasons. Losing them is the
    // one thing this migration must not do.
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
expect = "x"
why = ""
levers = [ { file = "f.nix", sed = "s|a|b|" } ]
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(problems.iter().any(|p| p.contains("why")), "{problems:?}");
}

// --- helpers ---------------------------------------------------------------

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "tamper-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn one_case(id: &str) -> String {
    one_case_for(id, "server")
}

fn one_case_for(id: &str, target: &str) -> String {
    format!(
        r#"
[[case]]
id = "{id}"
name = "x"
target = "{target}"
expect = "x"
why = "x"
levers = [ {{ file = "f.nix", sed = "s|a|b|" }} ]
"#
    )
}
