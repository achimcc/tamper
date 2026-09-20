use std::path::PathBuf;

use tamper::parse;

// These four drive `nix-instantiate` itself, which a Nix build sandbox does
// not have. They are ignored there rather than faked: a test that quietly
// passes because its tool is missing proves nothing and says so to nobody.
// The full run is `cargo test -- --include-ignored`, and `cargo test`
// reports the four as ignored, so the gap is visible and not silent.

fn sandbox() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-parse-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "needs nix-instantiate; run with --include-ignored"]
fn healthy_nix_parses() {
    let dir = sandbox();
    std::fs::write(dir.join("a.nix"), "{ uidBasis = 200000; }\n").unwrap();
    assert_eq!(parse::check(&dir, &[PathBuf::from("a.nix")]).unwrap(), None);
}

#[test]
#[ignore = "needs nix-instantiate; run with --include-ignored"]
fn a_syntax_error_is_reported_with_the_file_name() {
    // Measured 2026-09-20: `uidBasis = "pick;` gives
    // "syntax error, unexpected '=', expecting ';'".
    let dir = sandbox();
    std::fs::write(dir.join("a.nix"), "{ uidBasis = \"pick; }\n").unwrap();
    let found = parse::check(&dir, &[PathBuf::from("a.nix")]).unwrap();
    let msg = found.expect("broken Nix must be reported");
    assert!(msg.contains("a.nix"), "{msg}");
}

#[test]
#[ignore = "needs nix-instantiate; run with --include-ignored"]
fn files_that_are_not_nix_are_skipped() {
    // A case may sabotage a .yaml blueprint or a .rs file; nix-instantiate
    // has nothing to say about those.
    let dir = sandbox();
    std::fs::write(dir.join("x.yaml"), "not: [valid\n").unwrap();
    assert_eq!(
        parse::check(&dir, &[PathBuf::from("x.yaml")]).unwrap(),
        None
    );
}

#[test]
#[ignore = "needs nix-instantiate; run with --include-ignored"]
fn a_deleted_file_is_skipped_and_not_an_error() {
    // Case 39 removes a file from the index; the path shows up in
    // `git status` but there is nothing left to parse.
    let dir = sandbox();
    assert_eq!(
        parse::check(&dir, &[PathBuf::from("weg.nix")]).unwrap(),
        None
    );
}

#[test]
#[ignore = "needs nix-instantiate; run with --include-ignored"]
fn a_free_variable_is_not_a_parse_error() {
    // Measured 2026-09-20: `nix-instantiate --parse` resolves variables
    // inside string interpolations, so a syntactically perfect file with a
    // free `pkgs` fails with "undefined variable". That is an EVALUATION
    // error; the file parses. Reporting it as broken-nix would call a valid
    // case broken — which is exactly the misattribution this verdict exists
    // to prevent, only pointing the other way.
    let dir = sandbox();
    std::fs::write(
        dir.join("a.nix"),
        "{ probe = \"${pkgs.coreutils}/bin/printf\"; }\n",
    )
    .unwrap();
    assert_eq!(parse::check(&dir, &[PathBuf::from("a.nix")]).unwrap(), None);
}
