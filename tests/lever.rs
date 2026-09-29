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

// --- the lever stays inside the throwaway tree (audit 3, CD-8) ------------
//
// A lever edits the tree it is handed, and nothing else. Up to 0.3.1 an
// absolute path or a `../` walked out of it — and `tamper dry` still said
// `ok`, so a mistyped case could edit the main working tree unnoticed.

/// A file next to the sandbox, i.e. OUTSIDE the tree a lever gets.
fn outside(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.with_extension("draussen");
    std::fs::write(&path, "unberuehrt\n").unwrap();
    path
}

fn untouched(path: &std::path::Path) {
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "unberuehrt\n",
        "a lever changed a file outside the tree"
    );
}

#[test]
fn a_sed_lever_on_an_absolute_path_is_refused() {
    let dir = sandbox();
    let out = outside(&dir);
    let err = apply(
        &Lever::Sed {
            file: out.to_string_lossy().into_owned(),
            sed: "s|unberuehrt|veraendert|".into(),
        },
        &dir,
    )
    .unwrap_err();
    assert!(err.contains("outside"), "{err}");
    untouched(&out);
}

#[test]
fn a_perl_lever_that_climbs_out_with_dotdot_is_refused() {
    let dir = sandbox();
    let out = outside(&dir);
    let name = out.file_name().unwrap().to_string_lossy().into_owned();
    let err = apply(
        &Lever::Perl {
            file: format!("../{name}"),
            perl: "s/unberuehrt/veraendert/".into(),
        },
        &dir,
    )
    .unwrap_err();
    assert!(err.contains("outside"), "{err}");
    untouched(&out);
}

#[test]
fn a_symlink_inside_the_tree_that_points_out_is_refused() {
    // Lexically harmless, really outside: only the resolved path tells.
    let dir = sandbox();
    let out = outside(&dir);
    std::os::unix::fs::symlink(&out, dir.join("link.nix")).unwrap();
    std::os::unix::fs::symlink(out.parent().unwrap(), dir.join("linkdir")).unwrap();
    let name = out.file_name().unwrap().to_string_lossy().into_owned();
    for file in ["link.nix".to_string(), format!("linkdir/{name}")] {
        let err = apply(
            &Lever::Perl {
                file: file.clone(),
                perl: "s/unberuehrt/veraendert/".into(),
            },
            &dir,
        )
        .unwrap_err();
        assert!(err.contains("outside"), "{file}: {err}");
    }
    untouched(&out);
}

#[test]
fn a_symlink_that_stays_inside_the_tree_is_fine() {
    let dir = sandbox();
    std::os::unix::fs::symlink("a.nix", dir.join("b.nix")).unwrap();
    apply(
        &Lever::Perl {
            file: "b.nix".into(),
            perl: "s/200000/5000/".into(),
        },
        &dir,
    )
    .unwrap();
}

#[test]
fn a_sed_expression_cannot_run_a_command_or_write_a_file() {
    // "Without a shell" only ever meant quoting. GNU sed's `e` starts one
    // anyway, and `w`/`r` reach any file. `--sandbox` turns all three off.
    let dir = sandbox();
    let marker = dir.with_extension("e-lief");
    for sed in [
        format!("1e touch {}", marker.display()),
        format!("s|uidBasis|x|e touch {}", marker.display()),
        format!("1w {}", marker.display()),
        format!("s|uidBasis|x|w {}", marker.display()),
        "1r /etc/hostname".to_string(),
    ] {
        let err = apply(
            &Lever::Sed {
                file: "a.nix".into(),
                sed: sed.clone(),
            },
            &dir,
        )
        .unwrap_err();
        assert!(!err.is_empty(), "{sed}");
        assert!(!marker.exists(), "`{sed}` ran: {}", marker.display());
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("a.nix")).unwrap(),
        "{ uidBasis = 200000; }\n"
    );
}
