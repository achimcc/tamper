use tamper::build::{Outcome, classify};

#[test]
fn a_green_build_is_green() {
    assert!(matches!(classify(0, ""), Outcome::Green));
}

#[test]
fn a_failed_build_keeps_its_output() {
    let out = "error: Failed assertions:\n- Gast ohne numerische UID-Basis\n";
    match classify(1, out) {
        Outcome::Red(text) => assert!(text.contains("UID-Basis")),
        other => panic!("expected Red, got {other:?}"),
    }
}

#[test]
fn lotse_exit_200_is_a_queue_timeout_and_201_is_the_network() {
    // Measured 2026-09-20 from `lotse run --help`:
    //   200  --max-wait passed
    //   201  every attempt failed with a network pattern in its output
    // Neither says anything about the tree. Counting them as a ruling is
    // exactly the mistake the shell driver made with `Could not resolve host`.
    assert!(matches!(classify(200, ""), Outcome::QueueTimeout));
    assert!(matches!(classify(201, ""), Outcome::Network(_)));
}

#[test]
fn a_network_failure_that_reaches_us_anyway_is_not_a_ruling() {
    // Belt and braces: lotse retries these and then exits 201, but if a run
    // ever reaches us with the raw code, the message still has to keep the
    // case out of the tally.
    // curl's wording, which nix passes on: the host name follows a colon.
    let out = "error: unable to download 'https://cache.nixos.org/…': \
               Could not resolve host: cache.nixos.org (6)";
    assert!(matches!(classify(1, out), Outcome::Network(_)));
}

#[test]
fn a_daemon_disconnect_is_the_network_too() {
    let out = "error: Nix daemon disconnected unexpectedly";
    assert!(matches!(classify(1, out), Outcome::Network(_)));
}

#[test]
fn an_assertion_message_mentioning_a_download_is_still_a_ruling() {
    // The network patterns must not swallow a real assertion. This one
    // contains the word "download" and nothing else.
    let out =
        "error: Failed assertions:\n- Plugin-URL ohne Hash, download waere unreproduzierbar\n";
    assert!(matches!(classify(1, out), Outcome::Red(_)));
}

#[test]
fn lotses_own_summary_line_does_not_make_it_a_network_case() {
    // Measured: every `lotse run` appends a line of its own to the output.
    // It must not be mistaken for build output.
    let out = "error: Failed assertions:\n- etwas\nlotse: exit=1 attempts=1 waited=0s ran=12s\n";
    assert!(matches!(classify(1, out), Outcome::Red(_)));
}

#[test]
fn could_not_resolve_hostname_is_not_the_network() {
    // nftables' wording for an invalid address literal — and, quoted, part
    // of a real homeserver assertion (check 174). Without the colon in the
    // pattern it was taken for DNS; see tests/antworten.rs for the real run.
    let out = "error: Failed assertions:\n- ruleset.conf:340: Could not resolve hostname: \
               Name or service not known\n";
    assert!(matches!(classify(1, out), Outcome::Red(_)));
}
