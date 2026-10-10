//! The ledger by itself: what it stores, what it refuses to store, and the
//! order it gives a run.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use tamper::cases::{Case, Compare, Lever};
use tamper::ledger::{self, Ledger};
use tamper::verdict::Verdict;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tamper-ledger-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn case(id: &str) -> Case {
    Case {
        id: id.into(),
        name: "x".into(),
        target: "server".into(),
        expect: "x".into(),
        compare: Compare::Regex,
        why: "x".into(),
        green: false,
        levers: vec![Lever::Sed {
            file: "a.conf".into(),
            sed: "s|a|b|".into(),
        }],
    }
}

const DAY: u64 = 86_400;
const NOW: u64 = 1_800_000_000;

#[test]
fn a_ruling_comes_back_as_it_went_in() {
    let book = Ledger::open(&tempdir("roundtrip")).unwrap();
    book.put(
        "174",
        Verdict::OtherMessage,
        "8b1c3b00",
        "error: one\nerror: two",
        NOW,
    )
    .unwrap();
    let e = book.get("174").unwrap();
    assert_eq!(e.verdict, Some(Verdict::OtherMessage));
    assert_eq!(e.stamp, NOW);
    assert_eq!(e.commit, "8b1c3b00");
    assert_eq!(e.detail, "error: one\nerror: two");
    assert!(e.is_standing_finding());
}

#[test]
fn every_verdict_survives_the_round_trip() {
    // The ledger stores the slug; a slug `from_slug` does not know would
    // turn a stored finding into "never ruled".
    for v in Verdict::ALL {
        assert_eq!(Verdict::from_slug(v.slug()), Some(v));
    }
}

#[test]
fn a_night_without_network_changes_nothing() {
    // Neither may a case look freshly judged, nor may the finding that
    // stood before be wiped by a run that learned nothing.
    let book = Ledger::open(&tempdir("network")).unwrap();
    book.put("1", Verdict::NotRed, "aaaa", "", NOW - 3 * DAY)
        .unwrap();
    book.put("1", Verdict::Network, "bbbb", "", NOW).unwrap();
    book.put("1", Verdict::QueueTimeout, "bbbb", "", NOW)
        .unwrap();
    let e = book.get("1").unwrap();
    assert_eq!(e.verdict, Some(Verdict::NotRed));
    assert_eq!(e.stamp, NOW - 3 * DAY);
    assert_eq!(e.commit, "aaaa");
}

#[test]
fn a_new_case_is_seen_once_and_keeps_that_date() {
    let book = Ledger::open(&tempdir("seen")).unwrap();
    let (a, b) = (case("a"), case("b"));
    assert_eq!(book.note_seen(&[&a], NOW - 5 * DAY).unwrap(), 1);
    // The second sighting must not make the case younger.
    assert_eq!(book.note_seen(&[&a, &b], NOW).unwrap(), 1);
    assert_eq!(book.get("a").unwrap().stamp, NOW - 5 * DAY);
    assert_eq!(book.get("a").unwrap().verdict, None);
    assert_eq!(book.get("b").unwrap().stamp, NOW);
}

#[test]
fn an_id_cannot_leave_the_directory_or_share_a_file() {
    let dir = tempdir("names");
    let book = Ledger::open(&dir.join("book")).unwrap();
    book.put("../ausbruch", Verdict::Ok, "c", "", NOW).unwrap();
    book.put("a/b", Verdict::Ok, "c", "", NOW).unwrap();
    book.put("a_b", Verdict::NotRed, "c", "", NOW).unwrap();
    // Nothing next to the ledger, three files inside it.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(dir.join("book")).unwrap().count(), 3);
    assert_eq!(book.get("a/b").unwrap().verdict, Some(Verdict::Ok));
    assert_eq!(book.get("a_b").unwrap().verdict, Some(Verdict::NotRed));
}

#[test]
fn a_damaged_entry_is_no_entry() {
    let dir = tempdir("damaged");
    let book = Ledger::open(&dir).unwrap();
    book.put("1", Verdict::Ok, "c", "", NOW).unwrap();
    for text in [
        "",
        "verdict ok\n",
        "verdict erfunden\nstamp 1\n",
        "stamp x\n",
    ] {
        std::fs::write(dir.join("1.entry"), text).unwrap();
        assert_eq!(book.get("1"), None, "{text:?}");
    }
}

#[test]
fn the_entries_of_deleted_cases_go() {
    let dir = tempdir("prune");
    let book = Ledger::open(&dir).unwrap();
    book.put("bleibt", Verdict::Ok, "c", "", NOW).unwrap();
    book.put("geloescht", Verdict::NotRed, "c", "", NOW)
        .unwrap();
    // A file that is not an entry is none of the ledger's business.
    std::fs::write(dir.join("notiz.txt"), "x").unwrap();
    let known: BTreeSet<String> = ["bleibt".to_string()].into();
    assert_eq!(book.prune(&known).unwrap(), 1);
    assert!(book.get("bleibt").is_some());
    assert!(book.get("geloescht").is_none());
    assert!(dir.join("notiz.txt").exists());
}

#[test]
fn never_ruled_first_then_findings_then_ok_by_age() {
    let book = Ledger::open(&tempdir("order")).unwrap();
    let ids = [
        "ok-alt",
        "ok-jung",
        "befund",
        "nie",
        "ohne-eintrag",
        "ok-mittel",
    ];
    let cases: Vec<Case> = ids.iter().map(|i| case(i)).collect();
    let refs: Vec<&Case> = cases.iter().collect();

    book.put("ok-alt", Verdict::Ok, "c", "", NOW - 9 * DAY)
        .unwrap();
    book.put("ok-mittel", Verdict::Ok, "c", "", NOW - 4 * DAY)
        .unwrap();
    book.put("ok-jung", Verdict::Ok, "c", "", NOW - DAY)
        .unwrap();
    // A finding from YESTERDAY still goes before an `ok` from nine days
    // ago: a repaired case should turn green the next night.
    book.put("befund", Verdict::NotRed, "c", "", NOW - DAY)
        .unwrap();
    book.note_seen(&[&cases[3]], NOW - 2 * DAY).unwrap();

    let order: Vec<&str> = ledger::oldest_first(&refs, &book)
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "ohne-eintrag",
            "nie",
            "befund",
            "ok-alt",
            "ok-mittel",
            "ok-jung"
        ]
    );
}

#[test]
fn the_state_names_the_gap_and_the_oldest_entry() {
    let book = Ledger::open(&tempdir("state")).unwrap();
    let cases: Vec<Case> = ["a", "b", "c", "d"].iter().map(|i| case(i)).collect();
    let refs: Vec<&Case> = cases.iter().collect();
    book.put("a", Verdict::Ok, "c", "", NOW - 2 * DAY).unwrap();
    book.put("b", Verdict::DeadLever, "cafe", "the lever", NOW - DAY)
        .unwrap();
    // Never ruled, and waiting the longest: THIS is the oldest entry.
    book.note_seen(&[&cases[2]], NOW - 30 * DAY).unwrap();
    book.note_seen(&[&cases[3]], NOW).unwrap();

    let s = ledger::state(&refs, &book, NOW);
    assert_eq!(s.cases, 4);
    assert_eq!(s.never_ruled, ["c", "d"]);
    assert_eq!(s.findings.len(), 1);
    assert_eq!(s.findings[0].id, "b");
    assert_eq!(s.findings[0].age_secs, DAY);
    assert_eq!(s.oldest_secs, Some(30 * DAY));
    assert_eq!(s.oldest_id.as_deref(), Some("c"));

    let text = ledger::metrics(&s, 7, NOW);
    for line in [
        "tamper_cases 4",
        "tamper_never_ruled 2",
        "tamper_standing_findings 1",
        "tamper_oldest_ruling_seconds 2592000",
        "tamper_ruled_this_run 7",
        "tamper_last_run_timestamp_seconds 1800000000",
    ] {
        assert!(text.lines().any(|l| l == line), "{line}\n{text}");
    }
}

#[test]
fn a_duration_needs_its_unit() {
    assert_eq!(
        ledger::parse_duration("170m"),
        Ok(Duration::from_secs(10_200))
    );
    assert_eq!(
        ledger::parse_duration("3h"),
        Ok(Duration::from_secs(10_800))
    );
    assert_eq!(ledger::parse_duration("0s"), Ok(Duration::ZERO));
    for bad in ["170", "", "m", "3 h", "1.5h", "-1m", "3d", "ä"] {
        assert!(ledger::parse_duration(bad).is_err(), "{bad:?}");
    }
}
