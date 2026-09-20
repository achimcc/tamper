//! The cases: file, change, expected message — and why.

use std::path::Path;

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
    /// The message the build must carry. Empty exactly when `green` is set.
    #[serde(default)]
    pub expect: String,
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
        match (case.green, case.expect.trim().is_empty()) {
            (true, false) => problems.push(format!(
                "{at}: `green` and `expect` at once — a case either names the message it \
                 expects or says the check must stay green, not both"
            )),
            (false, true) => problems.push(format!(
                "{at}: neither `expect` nor `green` — the case does not say what it expects"
            )),
            _ => {}
        }
        if case.why.trim().is_empty() {
            problems.push(format!("{at}: empty `why` — a case without a reason rots"));
        }
        if !cfg.target.contains_key(&case.target) {
            problems.push(format!("{at}: unknown target `{}`", case.target));
        }
        if !case.green
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
