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

use crate::cases::Compare;

/// Every run of whitespace becomes one space; the ends are trimmed.
pub fn normalise(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn matches(output: &str, expect: &str, how: Compare) -> bool {
    let haystack = normalise(output);
    match how {
        Compare::Regex => regex::RegexBuilder::new(expect)
            .case_insensitive(true)
            .build()
            .map(|re| re.is_match(&haystack))
            // A pattern that does not compile is caught by `cases::validate`
            // long before this; if one gets here it must not silently pass.
            .unwrap_or(false),
        Compare::Literal => haystack.contains(&normalise(expect)),
    }
}
