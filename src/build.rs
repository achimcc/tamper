//! Building the sabotaged tree, queued through lotse.
//!
//! tamper brings NO limiter and NO network retry of its own. lotse already
//! queues every evaluation against the other dozen sessions on this
//! workstation (measured: one evaluation of `server` grows to 12 GB, the
//! machine has 30), and it already retries on a network failure. What is
//! left for us is to keep its two "no ruling" exits apart from a real one.
//!
//! Measured 2026-09-20:
//!
//! ```text
//! lotse run --class=eval -- false               -> 1   (the real code passes through)
//! lotse run --class=eval -- sh -c '… exit 3'    -> 3   (both streams pass through)
//! lotse run --help                              -> 200 = --max-wait passed
//!                                                  201 = every attempt failed
//!                                                        with a network pattern
//! ```
//!
//! So the process exit code can be read directly; there is no need to parse
//! the `lotse: exit=…` summary line it appends to the output.

use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub enum Outcome {
    /// The build succeeded — the check does not fire.
    Green,
    /// The build failed; the output is kept for the message comparison.
    Red(String),
    /// lotse (or the pattern below) blamed the network. No ruling BY ITSELF:
    /// the output is kept, because the evaluation may have got as far as the
    /// assertion the case is waiting for — then it is a ruling after all
    /// (`run::verdict_of`).
    Network(String),
    /// The build never started. No ruling.
    QueueTimeout,
    /// tamper itself could not run the build.
    Failed(String),
}

/// Patterns lotse itself retries on. We recognise them a second time in case
/// a raw exit code ever reaches us, so a network failure can never be
/// counted as a finding.
///
/// `Could not resolve host:` WITH the colon, which is how curl — and nix
/// through it — words a DNS failure. Without it the pattern also matched
/// "Could not resolve hostname", and that phrase stands in the homeserver's
/// own assertion text: check 174 quotes the nftables error it guards against.
/// Every "network failure" lotse ever retried on that machine (21 retries,
/// 2026-09-22) was that quotation.
const NETWORK: [&str; 3] = [
    "Could not resolve host:",
    "unable to download",
    "daemon disconnected",
];

pub fn classify(exit: i32, output: &str) -> Outcome {
    match exit {
        0 => Outcome::Green,
        200 => Outcome::QueueTimeout,
        201 => Outcome::Network(output.to_string()),
        _ if NETWORK.iter().any(|p| output.contains(p)) => Outcome::Network(output.to_string()),
        _ => Outcome::Red(output.to_string()),
    }
}

pub fn run(tree: &Path, attr: &str, class: &str) -> Outcome {
    // `--class=<name>` in ONE word: a lone `eval` as a separate argument is
    // blocked by the worktree guard of the sessions that call this.
    let out = Command::new("lotse")
        .arg("run")
        .arg(format!("--class={class}"))
        .arg("--")
        .arg("nix")
        .arg("build")
        .arg("--no-link")
        .arg(attr)
        .current_dir(tree)
        .output();

    let out = match out {
        Ok(o) => o,
        Err(e) => return Outcome::Failed(format!("cannot run lotse: {e}")),
    };

    // Both streams: nix writes its assertions to stderr, lotse its summary
    // to stdout, and the message we are looking for may be in either.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    classify(out.status.code().unwrap_or(-1), &text)
}
