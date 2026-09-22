//! What the run says at the end, and with which exit code.

use crate::run::Outcome;
use crate::verdict::Verdict;

/// A target whose unsabotaged tree did not build. Its cases were not run:
/// while the clean tree is red, every one of them would report "failed with
/// the expected message" or something equally meaningless.
#[derive(Debug, Clone)]
pub struct Blocked {
    pub target: String,
    /// Why the baseline has no green: the first errors of the build, or
    /// "network" / "queue timed out".
    pub reason: String,
    /// How many of the selected cases belong to this target.
    pub cases: usize,
}

pub struct Summary {
    pub total: usize,
    pub ok: usize,
    pub findings: Vec<(String, Verdict, String)>,
    pub without_ruling: Vec<(String, Verdict)>,
    pub from_cache: usize,
    pub oldest_cache_days: Option<u64>,
    /// Targets whose baseline was red. Their cases are not in `total`.
    pub blocked: Vec<Blocked>,
}

impl Summary {
    pub fn of(outcomes: &[Outcome]) -> Summary {
        Summary {
            total: outcomes.len(),
            ok: outcomes.iter().filter(|o| o.verdict == Verdict::Ok).count(),
            findings: outcomes
                .iter()
                .filter(|o| o.verdict.is_finding())
                .map(|o| (o.case_id.clone(), o.verdict, o.detail.clone()))
                .collect(),
            without_ruling: outcomes
                .iter()
                .filter(|o| !o.verdict.is_ruling())
                .map(|o| (o.case_id.clone(), o.verdict))
                .collect(),
            from_cache: outcomes.iter().filter(|o| o.from_cache).count(),
            // Derived from the outcomes rather than passed in, so the age in
            // the report and the entries that produced it cannot disagree.
            oldest_cache_days: outcomes.iter().filter_map(|o| o.cache_age_days).max(),
            blocked: Vec::new(),
        }
    }

    pub fn with_blocked(mut self, blocked: Vec<Blocked>) -> Summary {
        self.blocked = blocked;
        self
    }

    /// A finding first — it is what a human must act on. Then a red
    /// baseline: some cases were not run at all, and that must never read
    /// as 0. Then cases without a ruling.
    pub fn exit_code(&self) -> u8 {
        if !self.findings.is_empty() {
            1
        } else if !self.blocked.is_empty() {
            3
        } else if !self.without_ruling.is_empty() {
            4
        } else {
            0
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();

        for (id, verdict, detail) in &self.findings {
            out.push_str(&format!("  {:<14} case {id}\n", verdict.slug()));
            // Indented here and nowhere else, so the detail cannot be read
            // as another finding when 525 of these stand under each other.
            for line in detail.lines() {
                out.push_str(&format!("                 {line}\n"));
            }
        }

        out.push_str(&format!(
            "\n{} of {} cases ok, {} finding(s)\n",
            self.ok,
            self.total,
            self.findings.len()
        ));

        // COVERAGE, not the hit count: a cache that hides how much of the
        // run it answered is a cache nobody can judge.
        out.push_str(&format!(
            "cache answered {} of {} cases",
            self.from_cache, self.total
        ));
        match self.oldest_cache_days {
            Some(days) => out.push_str(&format!("; oldest entry {days} day(s) old\n")),
            None => out.push('\n'),
        }

        if !self.without_ruling.is_empty() {
            out.push_str(&format!(
                "\n{} case(s) WITHOUT A RULING — this is not a result:\n",
                self.without_ruling.len()
            ));
            for (id, verdict) in &self.without_ruling {
                out.push_str(&format!("  {:<14} case {id}\n", verdict.slug()));
            }
        }

        for b in &self.blocked {
            out.push_str(&format!(
                "\nBASELINE RED for target `{}` — its {} case(s) were NOT run, \
                 because the clean tree does not build:\n",
                b.target, b.cases
            ));
            for line in b.reason.lines() {
                out.push_str(&format!("                 {line}\n"));
            }
        }
        out
    }
}
