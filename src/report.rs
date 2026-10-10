//! What the run says at the end, and with which exit code.

use crate::ledger::{Standing, State};
use crate::run::Outcome;
use crate::verdict::Verdict;

/// What the ledger adds to the report of a run that kept one.
#[derive(Debug, Clone)]
pub struct Book {
    /// The ledger AFTER the run, over the whole selection.
    pub state: State,
    /// Findings the ledger holds for cases this run did not rule again.
    /// They count like fresh ones: a finding does not go away because the
    /// run that found it is over.
    pub standing: Vec<Standing>,
    /// Cases this run really built and ruled (no cache hits).
    pub ruled_this_run: usize,
    /// Cases the run did not get to before its time was up.
    pub not_reached: usize,
    /// `--max-age`, in days.
    pub max_age_days: Option<u64>,
}

impl Book {
    /// Is the oldest entry older than allowed? This is the promise the
    /// weekly `--no-cache` run was meant to keep and never did: no ruling
    /// older than N days, and no case waiting longer than that for its first.
    pub fn overdue(&self) -> bool {
        match (self.max_age_days, self.state.oldest_secs) {
            (Some(days), Some(oldest)) => oldest > days.saturating_mul(86_400),
            _ => false,
        }
    }
}

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
    /// Present when the run kept a ledger.
    pub book: Option<Book>,
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
            book: None,
        }
    }

    pub fn with_book(mut self, book: Book) -> Summary {
        self.book = Some(book);
        self
    }

    pub fn with_blocked(mut self, blocked: Vec<Blocked>) -> Summary {
        self.blocked = blocked;
        self
    }

    /// A finding first — it is what a human must act on, whether this run
    /// found it or the ledger still holds it. Then a red baseline: some
    /// cases were not run at all, and that must never read as 0. Then an
    /// overdue ledger: a lasting gap outranks a night without network. Then
    /// cases without a ruling.
    pub fn exit_code(&self) -> u8 {
        let standing = self.book.as_ref().is_some_and(|b| !b.standing.is_empty());
        if !self.findings.is_empty() || standing {
            1
        } else if !self.blocked.is_empty() {
            3
        } else if self.book.as_ref().is_some_and(Book::overdue) {
            5
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

        if let Some(book) = &self.book {
            out.push_str(&render_book(book));
        }
        out
    }
}

fn days(secs: u64) -> u64 {
    secs / 86_400
}

fn render_book(book: &Book) -> String {
    let mut out = String::new();
    for s in &book.standing {
        out.push_str(&format!(
            "\nSTANDING FINDING — ruled {} day(s) ago at {}, not ruled again in this run:\n  {:<14} case {}\n",
            days(s.age_secs),
            s.commit.get(..8).unwrap_or(&s.commit),
            s.verdict.slug(),
            s.id
        ));
        for line in s.detail.lines() {
            out.push_str(&format!("                 {line}\n"));
        }
    }

    // THE GAP, again: how much of the inventory this run did NOT judge, and
    // how old the oldest ruling is — never just "n cases ok".
    out.push_str(&format!(
        "\nledger: {} case(s); {} ruled in this run, {} not reached; {} never ruled",
        book.state.cases,
        book.ruled_this_run,
        book.not_reached,
        book.state.never_ruled.len()
    ));
    match (&book.state.oldest_id, book.state.oldest_secs) {
        (Some(id), Some(secs)) => out.push_str(&format!(
            "; oldest entry {} day(s) old (case {id})\n",
            days(secs)
        )),
        _ => out.push('\n'),
    }
    if book.overdue() {
        out.push_str(&format!(
            "OVERDUE: the oldest entry is older than the {} day(s) allowed — \
             the runs do not get round the inventory in time.\n",
            book.max_age_days.unwrap_or(0)
        ));
    }
    out
}
