use tamper::cases::{Compare, Lever};
use tamper::import::parse_script;

#[test]
fn a_plain_case_becomes_a_case_with_its_comment_as_why() {
    let text = r#"
# 2. `pick` statt einer Zahl — der Rueckfall aus Entscheidung 12.
sed -i '0,/uidBasis = 200000;/s//uidBasis = "pick";/' lib/gaeste.nix
faellt_mit "UID-Basis ist keine Zahl (pick-Rueckfall)" "ohne numerische UID-Basis"
"#;
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    assert_eq!(got.cases.len(), 1);
    let c = &got.cases[0];
    assert_eq!(c.id, "2");
    assert_eq!(c.name, "UID-Basis ist keine Zahl (pick-Rueckfall)");
    assert_eq!(c.expect, "ohne numerische UID-Basis");
    assert_eq!(c.target, "server");
    assert!(c.why.contains("Rueckfall aus Entscheidung 12"), "{}", c.why);
    match &c.levers[0] {
        Lever::Sed { file, sed } => {
            assert_eq!(file, "lib/gaeste.nix");
            // The shell quotes are gone: the expression reaches sed as one
            // argv element, so it needs no escaping of its own.
            assert_eq!(sed, r#"0,/uidBasis = 200000;/s//uidBasis = "pick";/"#);
        }
        other => panic!("expected a sed lever, got {other:?}"),
    }
}

#[test]
fn the_third_argument_is_the_target() {
    let text = r#"
# 7. Ein vHost ohne zweite Adresse.
sed -i 's|a|b|' hosts/vps/ingress.nix
faellt_mit "vHost ohne :8443" "ohne zweite Adresse" vps
"#;
    let got = parse_script(text);
    assert_eq!(got.cases[0].target, "vps");
}

#[test]
fn a_router_case_keeps_literal_comparison() {
    // router_fall uses grep -qF, faellt_mit uses grep -qi. Porting one as
    // the other would change verdicts silently.
    let text = r#"
# 11e. Ein ungueltiger Wert in der Firewall.
router_fall hosts/router/konfig.nix "Router: ungueltige Zonen-Policy" "PROBE-NEIN: fw4 check" 'print("x")'
"#;
    let got = parse_script(text);
    assert_eq!(got.cases.len(), 1, "{:?}", got.leftovers);
    assert_eq!(got.cases[0].target, "router-probe");
    assert_eq!(got.cases[0].compare, Compare::Literal);
    assert_eq!(got.cases[0].expect, "PROBE-NEIN: fw4 check");
}

#[test]
fn a_case_whose_lever_is_not_understood_lands_in_the_leftovers_by_name() {
    // The one thing the importer must not do is drop a case quietly.
    let text = r#"
# 39. inventar.env aus dem Index.
git rm -q --cached scripts/inventar.env
faellt_mit "inventar.env fehlt im Git-Tree" "inventar.env"
"#;
    let got = parse_script(text);
    assert!(got.cases.is_empty());
    assert_eq!(got.leftovers.len(), 1);
    assert!(
        got.leftovers[0].snippet.contains("git rm"),
        "{:?}",
        got.leftovers[0]
    );
    assert!(
        got.leftovers[0].reason.contains("39"),
        "{:?}",
        got.leftovers[0]
    );
}

#[test]
fn a_faellt_mit_without_any_lever_is_the_worse_leftover() {
    // A dead lever changes nothing; a MISSING lever means the case builds an
    // unchanged tree and reports green — it proves the opposite of its claim.
    let text = r#"
# 5. Eine Versions-Untergrenze.
faellt_mit "Versions-Untergrenze unterschritten" "Untergrenzen unterschritten"
"#;
    let got = parse_script(text);
    assert!(got.cases.is_empty());
    assert!(
        got.leftovers[0].reason.contains("no lever"),
        "{:?}",
        got.leftovers[0]
    );
}

#[test]
fn braces_are_escaped_so_the_pattern_still_compiles() {
    // Measured 2026-09-20: this engine refuses `{EntryData}` outright
    // ("repetition quantifier expects a valid decimal"), while grep's BRE
    // reads the braces literally. Escaping is the faithful port — it keeps
    // the case-insensitivity that `grep -qi` had.
    let text = r#"
# 99. Eine Vorlage ohne Marke.
sed -i 's|a|b|' modules/vorlagen.nix
faellt_mit "Vorlage ohne EntryData" "fehlt {EntryData}"
"#;
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    assert_eq!(got.cases[0].expect, r"fehlt \{EntryData\}");
}

#[test]
fn a_pattern_that_cannot_be_rescued_is_flagged_and_not_swallowed() {
    let text = r#"
# 98. Ein Muster mit offener Klammer.
sed -i 's|a|b|' modules/x.nix
faellt_mit "kaputtes Muster" "oh a( weh"
"#;
    let got = parse_script(text);
    assert!(got.cases.is_empty());
    assert!(
        got.leftovers[0].reason.contains("expect"),
        "{:?}",
        got.leftovers[0]
    );
}

#[test]
fn a_continuation_line_is_joined_before_parsing() {
    let text = "
# 12. Ein Fall ueber zwei Zeilen.
sed -i 's|a|b|' lib/gaeste.nix
faellt_mit \"Name des Falls\" \\
  \"die erwartete Meldung\"
";
    let got = parse_script(text);
    assert_eq!(got.cases.len(), 1, "{:?}", got.leftovers);
    assert_eq!(got.cases[0].expect, "die erwartete Meldung");
}

#[test]
fn several_levers_belong_to_the_case_that_follows_them() {
    let text = r#"
# 20. Zwei Hebel, ein Fall.
sed -i 's|a|b|' lib/gaeste.nix
sed -i 's|c|d|' checks.nix
faellt_mit "zwei Hebel" "eine Meldung"
"#;
    let got = parse_script(text);
    assert_eq!(got.cases[0].levers.len(), 2);
}

#[test]
fn a_quoted_argument_may_span_many_lines() {
    // All eight router_fall calls carry their python lever as a single-quoted
    // string over several real newlines. Joining only backslash-continuations
    // left them unparseable — and because `router_fall` is not a mutating
    // command, they were not even reported as leftovers. Eight cases gone
    // without a trace, which is the one thing this importer must never do.
    let text = "
# 11e. Ein ungueltiger Wert in der Firewall.
router_fall hosts/router/konfig.nix \"Router: ungueltige Zonen-Policy\" \\
  \"PROBE-NEIN: fw4 check\" '
import sys; p = sys.argv[1]; s = open(p).read()
open(p, \"w\").write(s.replace(\"REJECT\", \"FOO\"))'
";
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    assert_eq!(got.cases.len(), 1);
    assert_eq!(got.cases[0].id, "11e");
    assert_eq!(got.cases[0].target, "router-probe");
    assert_eq!(got.cases[0].compare, Compare::Literal);
    match &got.cases[0].levers[0] {
        Lever::Script { script, files } => {
            assert!(script.contains("import sys"), "{script}");
            assert_eq!(files, &vec!["hosts/router/konfig.nix".to_string()]);
        }
        other => panic!("expected a script lever, got {other:?}"),
    }
}

#[test]
fn an_argument_that_never_closes_is_reported_and_not_skipped() {
    let text = "
# 90. Ein Hebel mit offenem Anfuehrungszeichen.
python3 -c 'das hoert nie auf
faellt_mit \"x\" \"y\"
";
    let got = parse_script(text);
    assert!(got.cases.is_empty());
    assert!(!got.leftovers.is_empty(), "an unclosed quote must be named");
}

#[test]
fn case_numbers_grew_more_varied_than_digits_plus_one_letter() {
    // Measured on the real script: reading only `[0-9]+[a-z]?\.` left 258 of
    // 505 cases without a number. `9k2.` is a real heading, and
    // `fallkoepfe-pruefen.sh` does not see it either.
    for (head, want) in [
        ("# 2. Ein Fall.", "2"),
        ("# 9b. Noch einer.", "9b"),
        ("# 198b. Ein spaeter Nachtrag.", "198b"),
        ("# 9k2. Die Grundmenge.", "9k2"),
        ("# 11e. Der Router.", "11e"),
    ] {
        let text = format!("{head}\nsed -i 's|a|b|' f.nix\nfaellt_mit \"n\" \"m\"\n");
        let got = parse_script(&text);
        assert_eq!(got.cases[0].id, want, "for heading {head}");
    }
}

#[test]
fn a_sed_with_several_expressions_becomes_several_levers() {
    // Measured in the real script: `sed -i -e '…' -e '…' <file>` is a lever
    // form too. Applied in order, several Sed levers on one file do exactly
    // what one multi-expression sed did.
    let text = r#"
# 30. Zwei Ersetzungen in einer Datei.
sed -i -e 's|a|b|' -e 's|c|d|' lib/jellyfin-themes.nix
faellt_mit "zwei Ersetzungen" "eine Meldung"
"#;
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    assert_eq!(got.cases[0].levers.len(), 2);
    for l in &got.cases[0].levers {
        match l {
            Lever::Sed { file, .. } => assert_eq!(file, "lib/jellyfin-themes.nix"),
            other => panic!("expected sed levers, got {other:?}"),
        }
    }
}

#[test]
fn perl_flags_may_be_spelled_several_ways() {
    // Both `perl -0pi -e` and `perl -0 -i -pe` occur in the real script.
    let text = r#"
# 31. Mehrzeilige Ersetzung.
perl -0 -i -pe 's|a|b|' lib/jellyfin-plugins.nix
faellt_mit "mehrzeilig" "eine Meldung"
"#;
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    match &got.cases[0].levers[0] {
        Lever::Perl { file, perl } => {
            assert_eq!(file, "lib/jellyfin-plugins.nix");
            assert_eq!(perl, "s|a|b|");
        }
        other => panic!("expected a perl lever, got {other:?}"),
    }
}

#[test]
fn a_sed_without_minus_i_changes_no_file_and_is_not_a_leftover() {
    // `sed 's/^/  /' <<<"$x"` formats output. Counting it as an unrecognised
    // mutation would suppress a perfectly good case.
    let text = r#"
# 32. Ein Fall mit einer Ausgabe davor.
sed 's/^/        /' <<<"$abdeckung"
sed -i 's|a|b|' lib/gaeste.nix
faellt_mit "trotz Ausgabe" "eine Meldung"
"#;
    let got = parse_script(text);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
    assert_eq!(got.cases.len(), 1);
    assert_eq!(got.cases[0].levers.len(), 1);
}

#[test]
fn truncating_a_file_with_the_colon_builtin_is_named_as_a_mutation() {
    // `: > file` empties a file. Case 62 does exactly this, and calling it
    // "no lever at all" would point at the wrong thing.
    let text = r#"
# 62. Die Datei wird GELEERT, nicht geloescht.
: > hosts/server/gaeste/auth-01/35-selbstbedienung.yaml
faellt_mit "Selbstbedienung ohne Formular" "Kein Blueprint ueberschreibt die Stufe"
"#;
    let got = parse_script(text);
    assert!(got.cases.is_empty());
    assert!(
        got.leftovers[0].reason.contains("mutating"),
        "{:?}",
        got.leftovers[0]
    );
}

#[test]
fn output_and_control_flow_are_not_mistaken_for_levers() {
    // The script prints headings and has structure; neither is a mutation.
    let text = r#"
printf '\n\033[1mKann jede Pruefung rot werden?\033[0m\n\n'
# 1. Ein Dataset ohne Snapshot-Eintrag.
sed -i 's|a|b|' disko.nix
faellt_mit "Dataset ohne Snapshot-Abdeckung" "ohne Snapshot-Abdeckung"
"#;
    let got = parse_script(text);
    assert_eq!(got.cases.len(), 1);
    assert!(got.leftovers.is_empty(), "{:?}", got.leftovers);
}
