//! The seven outcomes of a sabotage case.
//!
//! The point of this enum is a distinction the shell driver could not make:
//! two of the seven say nothing at all about the tree (the network was gone,
//! or the build never started), and two more can be decided without building
//! anything. Collapsing them into "OK/FAIL" is what made a dead lever look
//! like a check that cannot go red.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The build failed and the expected message was in its output.
    Ok,
    /// The lever changed nothing — the case proves nothing. No build needed.
    DeadLever,
    /// The build was green: the check does not fire.
    NotRed,
    /// The build failed, but with a different message.
    OtherMessage,
    /// The sabotaged file no longer parses, so the parser failed, not the
    /// assertion. No build needed.
    BrokenNix,
    /// DNS or a download was gone. Not a ruling.
    Network,
    /// The build never started (the queue timed out). Not a ruling.
    QueueTimeout,
}

impl Verdict {
    pub const ALL: [Verdict; 7] = [
        Verdict::Ok,
        Verdict::DeadLever,
        Verdict::NotRed,
        Verdict::OtherMessage,
        Verdict::BrokenNix,
        Verdict::Network,
        Verdict::QueueTimeout,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::DeadLever => "dead-lever",
            Verdict::NotRed => "not-red",
            Verdict::OtherMessage => "other-message",
            Verdict::BrokenNix => "broken-nix",
            Verdict::Network => "network",
            Verdict::QueueTimeout => "queue-timeout",
        }
    }

    /// Something is wrong and a human has to look. Drives exit code 1.
    pub fn is_finding(self) -> bool {
        matches!(
            self,
            Verdict::DeadLever | Verdict::NotRed | Verdict::OtherMessage | Verdict::BrokenNix
        )
    }

    /// We learned something about the tree. `network` and `queue-timeout`
    /// did not, and must never be counted as either good or bad news.
    pub fn is_ruling(self) -> bool {
        !matches!(self, Verdict::Network | Verdict::QueueTimeout)
    }

    pub fn explain(self) -> &'static str {
        match self {
            Verdict::Ok => "the build failed with the expected message",
            Verdict::DeadLever => {
                "the lever changed nothing — the CASE is dead, not the check \
                 (checked with git diff, git diff --cached and git status --porcelain, \
                 because a case may stage or unstage a file)"
            }
            Verdict::NotRed => "the build was green — the check does not fire",
            Verdict::OtherMessage => "the build failed, but with a different message",
            Verdict::BrokenNix => {
                "the sabotaged file no longer parses — this proves that broken Nix \
                 does not build, not that the assertion fires"
            }
            Verdict::Network => "DNS or a download was gone — no ruling",
            Verdict::QueueTimeout => "the build never started (lotse queue) — no ruling",
        }
    }
}
