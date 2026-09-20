use tamper::cases::Lever;
use tamper::lever::apply;

fn sandbox() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-lever-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.nix"), "{ uidBasis = 200000; }\n").unwrap();
    dir
}

#[test]
fn a_sed_lever_edits_the_file() {
    let dir = sandbox();
    apply(
        &Lever::Sed {
            file: "a.nix".into(),
            sed: "0,/uidBasis = 200000;/s//uidBasis = 5000;/".into(),
        },
        &dir,
    )
    .unwrap();
    let text = std::fs::read_to_string(dir.join("a.nix")).unwrap();
    assert!(text.contains("uidBasis = 5000;"), "{text}");
}

#[test]
fn a_sed_expression_with_quotes_needs_no_escaping() {
    // The expression goes to sed as ONE argv element. There is no shell in
    // between, so the quoting bug that cost the palette case cannot happen.
    let dir = sandbox();
    std::fs::write(dir.join("a.nix"), "{ farbe = \"rot\"; }\n").unwrap();
    apply(
        &Lever::Sed {
            file: "a.nix".into(),
            sed: "s|\"rot\"|\"#ff0000\"|".into(),
        },
        &dir,
    )
    .unwrap();
    let text = std::fs::read_to_string(dir.join("a.nix")).unwrap();
    assert!(text.contains("\"#ff0000\""), "{text}");
}

#[test]
fn a_perl_lever_can_span_lines() {
    let dir = sandbox();
    std::fs::write(dir.join("a.nix"), "{\n  a = 1;\n  b = 2;\n}\n").unwrap();
    apply(
        &Lever::Perl {
            file: "a.nix".into(),
            perl: "s/a = 1;\\n  b = 2;/a = 9;/s".into(),
        },
        &dir,
    )
    .unwrap();
    let text = std::fs::read_to_string(dir.join("a.nix")).unwrap();
    assert!(text.contains("a = 9;"), "{text}");
    assert!(!text.contains("b = 2;"), "{text}");
}

#[test]
fn a_script_lever_runs_in_the_tree() {
    let dir = sandbox();
    apply(
        &Lever::Script {
            script: "printf '{ }\\n' > neu.nix".into(),
            files: vec!["neu.nix".into()],
        },
        &dir,
    )
    .unwrap();
    assert!(dir.join("neu.nix").exists());
}

#[test]
fn a_failing_script_lever_is_an_error_and_not_a_dead_lever() {
    // A lever that CRASHED is a broken case, and it must not be reported as
    // "the lever changed nothing" — that would point at the check instead of
    // at the case.
    let dir = sandbox();
    let err = apply(
        &Lever::Script {
            script: "exit 7".into(),
            files: vec!["a.nix".into()],
        },
        &dir,
    )
    .unwrap_err();
    assert!(err.contains('7'), "{err}");
}

#[test]
fn a_sed_on_a_missing_file_is_an_error() {
    let dir = sandbox();
    let err = apply(
        &Lever::Sed {
            file: "gibtsnicht.nix".into(),
            sed: "s|a|b|".into(),
        },
        &dir,
    )
    .unwrap_err();
    assert!(err.contains("gibtsnicht.nix"), "{err}");
}
