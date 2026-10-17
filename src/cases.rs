//! The cases: file, change, expected message — and why.

use std::path::{Component, Path};

use serde::Deserialize;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Compare {
    /// Case-insensitive regular expression, like `grep -qi`. The default,
    /// because the patterns use metacharacters on purpose.
    #[default]
    Regex,
    /// Case-sensitive substring, like `grep -qF` in `router_fall`.
    Literal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lever {
    /// `sed -i <expr> <file>` — run without a shell, so nothing has to be quoted.
    Sed { file: String, sed: String },
    /// `perl -0pi -e <expr> <file>` — the multi-line cases.
    Perl { file: String, perl: String },
    /// A bash snippet for everything else (git, cat, python3, heredocs).
    /// `files` is REQUIRED: the cache key is computed before the lever runs.
    Script { script: String, files: Vec<String> },
}

impl Lever {
    pub fn files(&self) -> Vec<String> {
        match self {
            Lever::Sed { file, .. } | Lever::Perl { file, .. } => vec![file.clone()],
            Lever::Script { files, .. } => files.clone(),
        }
    }
}

/// Deserialised by hand rather than with `#[serde(untagged)]`, because an
/// untagged enum answers every mistake with "data did not match any variant".
/// The whole point of this file is to say which key is missing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLever {
    file: Option<String>,
    sed: Option<String>,
    perl: Option<String>,
    script: Option<String>,
    files: Option<Vec<String>>,
}

impl TryFrom<RawLever> for Lever {
    type Error = String;

    fn try_from(raw: RawLever) -> Result<Lever, String> {
        match (raw.sed, raw.perl, raw.script) {
            (Some(sed), None, None) => Ok(Lever::Sed {
                file: raw
                    .file
                    .ok_or("a `sed` lever needs a `file` to work on".to_string())?,
                sed,
            }),
            (None, Some(perl), None) => Ok(Lever::Perl {
                file: raw
                    .file
                    .ok_or("a `perl` lever needs a `file` to work on".to_string())?,
                perl,
            }),
            (None, None, Some(script)) => {
                let files = raw.files.ok_or_else(|| {
                    "a `script` lever must declare the `files` it touches — the cache key \
                     is computed before the lever runs, so it cannot be asked afterwards"
                        .to_string()
                })?;
                if files.is_empty() {
                    return Err("`files` is empty; name what the script touches".into());
                }
                Ok(Lever::Script { script, files })
            }
            (None, None, None) => Err("a lever needs one of `sed`, `perl` or `script`".to_string()),
            _ => Err("a lever has exactly one of `sed`, `perl` or `script`, not several".into()),
        }
    }
}

impl<'de> Deserialize<'de> for Lever {
    fn deserialize<D>(d: D) -> Result<Lever, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLever::deserialize(d)?;
        Lever::try_from(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub name: String,
    pub target: String,
    /// The message the build must carry. May be empty when `green` is set or
    /// `checks` names the assertions instead.
    #[serde(default)]
    pub expect: String,
    /// The ids of the checks this case must bring down, e.g. `P-345`. The
    /// build output must carry the marker `[P-345]` for every one of them,
    /// literally. A check id is a VALUE where `expect` is a pattern: the
    /// pattern `Pruefung 103` also matches `Pruefung 103b`, and a case that
    /// brought down the wrong check came back `ok`. May stand alone or next
    /// to `expect`; then both must hold.
    #[serde(default)]
    pub checks: Vec<String>,
    /// This change must leave the check GREEN. A handful of cases state that
    /// — they guard against a check that fires on something legitimate — and
    /// it is the opposite statement from every other case here.
    #[serde(default)]
    pub green: bool,
    #[serde(default)]
    pub compare: Compare,
    pub why: String,
    pub levers: Vec<Lever>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    #[serde(default, rename = "case")]
    cases: Vec<Case>,
}

pub fn load_dir(dir: &Path) -> Result<Vec<Case>, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();

    let mut out = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let parsed: CaseFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        out.extend(parsed.cases);
    }
    Ok(out)
}

/// A lever may only name a path INSIDE the tree: relative, and without `..`.
/// Checked when the cases are loaded, so `tamper list` says so before any
/// tree exists; `lever::apply` checks again on the resolved path, which is
/// the only place a symlink shows (audit 3, CD-8).
pub fn check_lever_path(file: &str) -> Result<(), String> {
    if file.is_empty() {
        return Err("a lever names an empty path".into());
    }
    let escapes = Path::new(file).components().any(|c| {
        matches!(
            c,
            Component::RootDir | Component::Prefix(_) | Component::ParentDir
        )
    });
    if escapes {
        return Err(format!(
            "lever path {file} leads outside the throwaway tree — only relative \
             paths without `..` are allowed"
        ));
    }
    Ok(())
}

/// A check id: `P-`, then lowercase letters, digits and hyphens, starting
/// with a letter or digit — `^P-[a-z0-9][a-z0-9-]*$`. Spelled out rather than
/// compiled, so the error can say which part is wrong.
pub fn check_id(id: &str) -> Result<(), String> {
    let wrong = |why: &str| {
        Err(format!(
            "`{id}` is not a check id ({why}) — a check id is `P-` followed by lowercase \
             letters, digits and hyphens, e.g. `P-345` or `P-ryot-zusagen`"
        ))
    };
    let Some(rest) = id.strip_prefix("P-") else {
        return wrong("it does not start with `P-`");
    };
    let Some(first) = rest.chars().next() else {
        return wrong("nothing follows `P-`");
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return wrong("the part after `P-` must start with a lowercase letter or a digit");
    }
    if !rest
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return wrong("only lowercase letters, digits and hyphens are allowed after `P-`");
    }
    Ok(())
}

/// Everything that can be decided without touching the repository. Returns
/// one line per problem; empty means the set is sound.
pub fn validate(cases: &[Case], cfg: &Config) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    for case in cases {
        let at = format!("case {}", case.id);

        if seen.contains(&case.id.as_str()) {
            problems.push(format!("{at}: id handed out twice"));
        }
        seen.push(&case.id);

        if case.levers.is_empty() {
            problems.push(format!(
                "{at}: no lever — the case would build an unchanged tree and report green"
            ));
        }
        // What the case expects: the check must stay GREEN, or the build
        // must go red with this message (`expect`), or bringing down these
        // checks (`checks`), or both of the latter. GREEN contradicts either.
        let has_expect = !case.expect.trim().is_empty();
        let has_checks = !case.checks.is_empty();
        if case.green && has_expect {
            problems.push(format!(
                "{at}: `green` and `expect` at once — a case either names the message it \
                 expects or says the check must stay green, not both"
            ));
        }
        if case.green && has_checks {
            problems.push(format!(
                "{at}: `green` and `checks` at once — a case either names the checks it \
                 brings down or says the check must stay green, not both"
            ));
        }
        if !case.green && !has_expect && !has_checks {
            problems.push(format!(
                "{at}: neither `expect`, `checks` nor `green` — the case does not say what \
                 it expects"
            ));
        }
        for id in &case.checks {
            if let Err(e) = check_id(id) {
                problems.push(format!("{at}: `checks`: {e}"));
            }
        }
        for file in case.levers.iter().flat_map(Lever::files) {
            if let Err(e) = check_lever_path(&file) {
                problems.push(format!("{at}: {e}"));
            }
        }
        if case.why.trim().is_empty() {
            problems.push(format!("{at}: empty `why` — a case without a reason rots"));
        }
        if !cfg.target.contains_key(&case.target) {
            problems.push(format!("{at}: unknown target `{}`", case.target));
        }
        if !case.green
            && has_expect
            && case.compare == Compare::Regex
            && let Err(e) = regex::RegexBuilder::new(&case.expect)
                .case_insensitive(true)
                .build()
        {
            problems.push(format!("{at}: `expect` is not a valid regex: {e}"));
        }
    }
    problems
}
