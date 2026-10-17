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

#[test]
fn a_case_may_demand_that_the_check_stays_green() {
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "129c"
name = "Gequotetes Here-Dokument bleibt gruen"
target = "server"
green = true
why = "Die Pruefung darf hier NICHT anschlagen."
levers = [ { file = "f.nix", sed = "s|a|b|" } ]
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    assert!(cases[0].green);
    assert!(cases::validate(&cases, &config()).is_empty());
}

#[test]
fn green_and_expect_are_mutually_exclusive() {
    // A case either names the message it expects, or it says the check must
    // stay green. Both at once means nobody decided which.
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
green = true
expect = "eine Meldung"
why = "x"
levers = [ { file = "f.nix", sed = "s|a|b|" } ]
"#,
    );
    let cases = cases::load_dir(&dir).unwrap();
    let problems = cases::validate(&cases, &config());
    assert!(problems.iter().any(|p| p.contains("green")), "{problems:?}");
}

#[test]
fn a_case_without_green_needs_an_expect() {
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
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
fn a_lever_path_outside_the_tree_is_an_error() {
    // Absolute paths and `..` are refused when the cases are loaded, so
    // `tamper list` says so before any tree exists (audit 3, CD-8).
    for (file, lever) in [
        ("/etc/passwd", r#"{ file = "/etc/passwd", sed = "s|a|b|" }"#),
        (
            "../draussen.nix",
            r#"{ file = "../draussen.nix", perl = "s/a/b/" }"#,
        ),
        (
            "lib/../../x.nix",
            r#"{ file = "lib/../../x.nix", sed = "s|a|b|" }"#,
        ),
        (
            "../neu.nix",
            r#"{ script = "true", files = ["ok.nix", "../neu.nix"] }"#,
        ),
    ] {
        let dir = tempdir();
        write(
            &dir,
            "a.toml",
            &format!(
                r#"
[[case]]
id = "1"
name = "x"
target = "server"
expect = "x"
why = "x"
levers = [ {lever} ]
"#
            ),
        );
        let cases = cases::load_dir(&dir).unwrap();
        let problems = cases::validate(&cases, &config());
        assert!(
            problems
                .iter()
                .any(|p| p.contains(file) && p.contains("outside")),
            "{file}: {problems:?}"
        );
    }
}

#[test]
fn a_plain_relative_lever_path_is_fine() {
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        &one_case("1").replace("f.nix", "./lib/gaeste.nix"),
    );
    let cases = cases::load_dir(&dir).unwrap();
    assert!(cases::validate(&cases, &config()).is_empty());
}

// --- checks: the ids of the assertions a case brings down -------------------

fn case_toml(fields: &str) -> String {
    format!(
        r#"
[[case]]
id = "1"
name = "x"
target = "server"
{fields}
why = "x"
levers = [ {{ file = "f.nix", sed = "s|a|b|" }} ]
"#
    )
}

fn problems_of(fields: &str) -> Vec<String> {
    let dir = tempdir();
    write(&dir, "a.toml", &case_toml(fields));
    let cases = cases::load_dir(&dir).unwrap();
    cases::validate(&cases, &config())
}

#[test]
fn a_case_may_name_the_checks_it_brings_down_instead_of_a_message() {
    // `expect = "Pruefung 103"` also matches `Pruefung 103b`. An id is a
    // value, not a pattern: the case says WHICH check must fall.
    let dir = tempdir();
    write(
        &dir,
        "a.toml",
        &case_toml(r#"checks = ["P-103", "P-ryot-zusagen"]"#),
    );
    let cases = cases::load_dir(&dir).unwrap();
    assert_eq!(cases[0].checks, vec!["P-103", "P-ryot-zusagen"]);
    assert!(cases[0].expect.is_empty());
    assert!(cases::validate(&cases, &config()).is_empty());
}

#[test]
fn checks_and_expect_together_are_allowed() {
    // Then both must hold — the id says which check, the pattern what it says.
    let problems = problems_of(
        r#"checks = ["P-103"]
expect = "ohne Snapshot-Abdeckung""#,
    );
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn green_and_checks_contradict_each_other() {
    let problems = problems_of(
        r#"green = true
checks = ["P-103"]"#,
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("green") && p.contains("checks")),
        "{problems:?}"
    );
}

#[test]
fn an_empty_checks_list_does_not_count_as_an_expectation() {
    let problems = problems_of("checks = []");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("expect") && p.contains("checks") && p.contains("green")),
        "{problems:?}"
    );
}

#[test]
fn the_three_expectations_cover_every_combination() {
    // (green, expect, checks) -> sound?
    for (fields, sound) in [
        ("green = true", true),
        (r#"expect = "x""#, true),
        (r#"checks = ["P-1"]"#, true),
        ("checks = [\"P-1\"]\nexpect = \"x\"", true),
        ("green = true\nexpect = \"x\"", false),
        ("green = true\nchecks = [\"P-1\"]", false),
        ("green = true\nexpect = \"x\"\nchecks = [\"P-1\"]", false),
        ("", false),
    ] {
        let problems = problems_of(fields);
        assert_eq!(problems.is_empty(), sound, "{fields:?}: {problems:?}");
    }
}

#[test]
fn a_check_id_has_one_shape() {
    for bad in [
        "345",
        "p-345",
        "P-",
        "P-Ryot",
        "P-ryot zusagen",
        "P--x",
        "[P-345]",
        "P-ä",
    ] {
        let problems = problems_of(&format!("checks = [{bad:?}]"));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("checks") && p.contains(bad) && p.contains("P-")),
            "{bad}: {problems:?}"
        );
    }
    for good in ["P-345", "P-ryot-zusagen", "P-103b", "P-0", "P-a-"] {
        assert!(
            problems_of(&format!("checks = [{good:?}]")).is_empty(),
            "{good} should be a valid check id"
        );
        assert!(cases::check_id(good).is_ok(), "{good}");
    }
}

#[test]
fn an_invalid_check_id_says_which_part_is_wrong() {
    let err = cases::check_id("P-Ryot").unwrap_err();
    assert!(err.contains("lowercase"), "{err}");
    let err = cases::check_id("345").unwrap_err();
    assert!(err.contains("start with `P-`"), "{err}");
    let err = cases::check_id("P-").unwrap_err();
    assert!(err.contains("nothing follows"), "{err}");
}

#[test]
fn a_checks_only_case_does_not_have_its_empty_expect_compiled_as_a_regex() {
    // The empty pattern compiles and matches everything; it must neither be
    // reported nor relied on. `message::expected` ignores an empty `expect`.
    let problems = problems_of(r#"checks = ["P-1"]"#);
    assert!(problems.is_empty(), "{problems:?}");
}
