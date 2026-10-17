//! Comparing the build output against the expected message.
//!
//! Two decisions, both taken from the shell driver being replaced:
//!
//!  * The comparison runs against NORMALISED whitespace. Nix wraps its
//!    output, and a pattern spanning the break made a healthy check look
//!    dead.
//!  * The pattern is a REGULAR EXPRESSION by default, not a substring:
//!    30 of 522 patterns use metacharacters on purpose, several with a `.`
//!    standing in for a backtick.

//!  * A check id (`checks = ["P-345"]`) is looked for as the MARKER `[P-345]`,
//!    literally and case-sensitively, after the same normalisation. A marker
//!    carries no whitespace, so no line break can split it.

use crate::cases::{Case, Compare};

/// Every run of whitespace becomes one space; the ends are trimmed.
pub fn normalise(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text a check with this id puts in front of its message.
pub fn marker(id: &str) -> String {
    format!("[{id}]")
}

pub fn matches(output: &str, expect: &str, how: Compare) -> bool {
    matches_normalised(&normalise(output), expect, how)
}

/// The ids of `checks` whose marker is NOT in the output, in the order the
/// case names them. Empty means every named check fired.
pub fn missing_checks<'a>(output: &str, checks: &'a [String]) -> Vec<&'a str> {
    let haystack = normalise(output);
    checks
        .iter()
        .filter(|id| !haystack.contains(&marker(id)))
        .map(String::as_str)
        .collect()
}

/// Did the build output carry what the case waits for? Every check named in
/// `checks` must have left its marker, AND `expect` must match if the case
/// sets one. A case with neither is refused by `cases::validate` and never
/// gets here; should one, it must not pass.
pub fn expected(output: &str, case: &Case) -> bool {
    let haystack = normalise(output);
    let has_expect = !case.expect.trim().is_empty();
    if case.checks.is_empty() && !has_expect {
        return false;
    }
    let markers = case.checks.iter().all(|id| haystack.contains(&marker(id)));
    let text = !has_expect || matches_normalised(&haystack, &case.expect, case.compare);
    markers && text
}

fn matches_normalised(haystack: &str, expect: &str, how: Compare) -> bool {
    match how {
        Compare::Regex => regex::RegexBuilder::new(expect)
            .case_insensitive(true)
            .build()
            .map(|re| re.is_match(haystack))
            // A pattern that does not compile is caught by `cases::validate`
            // long before this; if one gets here it must not silently pass.
            .unwrap_or(false),
        Compare::Literal => haystack.contains(&normalise(expect)),
    }
}
