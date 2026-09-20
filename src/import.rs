//! One-shot: read the shell driver and hand back cases plus a leftover list.
//!
//! This subcommand exists for the migration and is meant to be deleted with
//! the script it reads. Two rules shape it:
//!
//!  * **Nothing is dropped quietly.** Every mutating line it cannot turn
//!    into a lever, and every `faellt_mit` without one, comes back as a
//!    named leftover with its line number. The lesson from
//!    `hebel-pruefen.py`, which named its gap only from 2026-09-10 on: a
//!    check that does not reach all of its subjects must report the GAP,
//!    not the hit count.
//!  * **Function bodies are skipped.** `faellt_mit` and `aufraeumen` are
//!    defined in the same file they are called from, and their bodies
//!    contain both a `faellt_mit` mention and a `git checkout`. A body runs
//!    from `name()` to a closing `}` at column 0.

use crate::cases::{Case, Compare, Lever};

#[derive(Debug)]
pub struct Leftover {
    pub line: usize,
    pub snippet: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Imported {
    pub cases: Vec<Case>,
    pub leftovers: Vec<Leftover>,
    /// How many `faellt_mit`/`router_fall` calls were seen at all. The sum
    /// of cases and their leftovers has to account for every one of them.
    pub seen_calls: usize,
    /// How many cases had no number in their comment block and got one made
    /// up from their name. Reported, never hidden: a made-up id is fine as a
    /// key and useless in a sentence like "see case 51".
    pub synthetic_ids: usize,
}

/// Commands that change a file. Anything else at column 0 is structure or
/// output, not a lever. `printf`, `echo` and `cat` only count when the line
/// redirects somewhere.
const MUTATING: [&str; 8] = [
    "python3", "python", "cp", "mv", "rm", "touch", "mkdir", "ln",
];
const MUTATING_IF_REDIRECTED: [&str; 6] = ["printf", "echo", "cat", "tee", ":", "true"];
const MUTATING_GIT: [&str; 6] = ["add", "rm", "mv", "apply", "checkout", "restore"];
/// `sed` and `perl` only touch a file with `-i`. Without it they format
/// output — and treating that as an unrecognised mutation would suppress a
/// perfectly good case, which is worse than the noise it saves.
const MUTATING_WITH_I: [&str; 2] = ["sed", "perl"];

pub fn parse_script(text: &str) -> Imported {
    let joined = text.replace("\\\n", " ");
    let mut out = Imported::default();

    let mut comment: Vec<String> = Vec::new();
    let mut levers: Vec<Lever> = Vec::new();
    let mut pending: Vec<Leftover> = Vec::new();
    let mut in_function = false;

    let zeilen: Vec<&str> = joined.lines().collect();
    let mut i = 0;
    while i < zeilen.len() {
        let no = i + 1;
        let line = zeilen[i].trim_end();
        i += 1;

        if in_function {
            if line.starts_with('}') {
                in_function = false;
            }
            continue;
        }
        if is_function_head(line) {
            in_function = true;
            continue;
        }

        if let Some(rest) = line.strip_prefix('#') {
            comment.push(rest.trim().to_string());
            continue;
        }
        if line.trim().is_empty() {
            // A blank line ends a comment block that belongs to nothing.
            if levers.is_empty() && pending.is_empty() {
                comment.clear();
            }
            continue;
        }

        // A QUOTED ARGUMENT MAY SPAN MANY LINES, and all eight `router_fall`
        // calls use that: their python lever is one single-quoted string over
        // several real newlines. Reading line by line left them unparseable —
        // and since `router_fall` is not a mutating command, they were not
        // even reported. Eight cases gone without a trace.
        let mut befehl = line.to_string();
        let mut argv = tokenize(&befehl);
        while argv.is_err() && i < zeilen.len() && i - no < 60 {
            befehl.push('\n');
            befehl.push_str(zeilen[i]);
            i += 1;
            argv = tokenize(&befehl);
        }

        let Ok(argv) = argv else {
            pending.push(Leftover {
                line: no,
                snippet: befehl.lines().next().unwrap_or("").trim().to_string(),
                reason: "cannot be tokenised: a quote never closes".into(),
            });
            continue;
        };
        let Some(head) = argv.first().map(String::as_str) else {
            continue;
        };

        match head {
            "faellt_mit" => {
                out.seen_calls += 1;
                finish(&mut out, &mut comment, &mut levers, &mut pending, &argv, no);
            }
            "router_fall" => {
                out.seen_calls += 1;
                finish_router(&mut out, &mut comment, &mut levers, &mut pending, &argv, no);
            }
            _ => {
                let found = as_levers(&argv);
                if !found.is_empty() {
                    levers.extend(found);
                } else if is_mutating(&befehl) {
                    pending.push(Leftover {
                        line: no,
                        snippet: befehl.lines().next().unwrap_or("").trim().to_string(),
                        reason: "mutating command in a form that is not a lever".into(),
                    });
                }
            }
        }
    }
    // WHAT IS LEFT OVER AT THE END IS STILL LEFT OVER. `pending` is drained
    // by the case that follows it; a mutating line with no case after it
    // would otherwise be dropped here, silently, in the very function whose
    // job is to drop nothing.
    out.leftovers.extend(pending);
    out
}

fn is_function_head(line: &str) -> bool {
    let name: String = line
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    !name.is_empty() && line[name.len()..].trim_start().starts_with("()")
}

fn is_mutating(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let head = words.first().copied().unwrap_or("");
    if MUTATING.contains(&head) {
        return true;
    }
    if MUTATING_IF_REDIRECTED.contains(&head) {
        return line.contains('>');
    }
    if MUTATING_WITH_I.contains(&head) {
        return words.iter().any(|w| w.starts_with('-') && w.contains('i'));
    }
    if head == "git" {
        let sub = words.get(1).copied().unwrap_or("");
        return MUTATING_GIT.contains(&sub);
    }
    false
}

/// Turn one command into levers, or nothing if it is not one.
///
/// Both forms are more generous than they first looked in the real script:
/// `sed -i -e '…' -e '…' <file>` carries several expressions, and perl
/// spells its flags as `-0pi -e` in some cases and `-0 -i -pe` in others.
/// Reading only the narrow form left five genuine levers sitting in the
/// leftover list.
fn as_levers(argv: &[String]) -> Vec<Lever> {
    match argv.first().map(String::as_str).unwrap_or("") {
        "sed" => sed_levers(argv),
        "perl" => perl_lever(argv).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn sed_levers(argv: &[String]) -> Vec<Lever> {
    let args = &argv[1..];
    if !args.iter().any(|a| a == "-i") {
        return Vec::new();
    }
    let Some(file) = args.last() else {
        return Vec::new();
    };
    if file.starts_with('-') {
        return Vec::new();
    }

    let mut exprs: Vec<String> = Vec::new();
    let body = &args[..args.len() - 1];
    let mut i = 0;
    while i < body.len() {
        match body[i].as_str() {
            "-e" => {
                if let Some(e) = body.get(i + 1) {
                    exprs.push(e.clone());
                }
                i += 2;
            }
            "-i" => i += 1,
            // The bare expression form: `sed -i '<expr>' <file>`.
            other if !other.starts_with('-') => {
                exprs.push(other.to_string());
                i += 1;
            }
            _ => return Vec::new(),
        }
    }
    exprs
        .into_iter()
        .map(|sed| Lever::Sed {
            file: file.clone(),
            sed,
        })
        .collect()
}

fn perl_lever(argv: &[String]) -> Option<Lever> {
    let args = &argv[1..];
    // In place, whichever way the flags are spelled.
    if !args
        .iter()
        .any(|a| a.starts_with('-') && a.contains('i') && !a.starts_with("--"))
    {
        return None;
    }
    // The expression follows the flag that ends in `e` (`-e`, `-pe`, `-0pe`).
    let at = args
        .iter()
        .position(|a| a.starts_with('-') && a.ends_with('e'))?;
    let perl = args.get(at + 1)?.clone();
    let file = args.last()?;
    if file.starts_with('-') || file == &perl {
        return None;
    }
    Some(Lever::Perl {
        file: file.clone(),
        perl,
    })
}

fn finish(
    out: &mut Imported,
    comment: &mut Vec<String>,
    levers: &mut Vec<Lever>,
    pending: &mut Vec<Leftover>,
    argv: &[String],
    line: usize,
) {
    let (id, why) = head_and_why(comment);
    let name = argv.get(1).cloned().unwrap_or_default();
    let expect = argv.get(2).cloned().unwrap_or_default();
    let target = argv.get(3).cloned().unwrap_or_else(|| "server".into());
    emit(
        out,
        levers,
        pending,
        Draft {
            id,
            name,
            expect,
            target,
            compare: Compare::Regex,
            why,
            line,
        },
    );
}

fn finish_router(
    out: &mut Imported,
    comment: &mut Vec<String>,
    levers: &mut Vec<Lever>,
    pending: &mut Vec<Leftover>,
    argv: &[String],
    line: usize,
) {
    // router_fall <file> <name> <pattern> <python lever> [check]
    let (id, why) = head_and_why(comment);
    let file = argv.get(1).cloned().unwrap_or_default();
    let name = argv.get(2).cloned().unwrap_or_default();
    let expect = argv.get(3).cloned().unwrap_or_default();
    let code = argv.get(4).cloned().unwrap_or_default();
    let target = argv
        .get(5)
        .cloned()
        .unwrap_or_else(|| "router-probe".into());

    levers.push(Lever::Script {
        script: format!("python3 -c {} {}", sh_quote(&code), sh_quote(&file)),
        files: vec![file],
    });
    emit(
        out,
        levers,
        pending,
        Draft {
            id,
            name,
            expect,
            target,
            // router_fall compares with `grep -qF`: literal, case-sensitive.
            compare: Compare::Literal,
            why,
            line,
        },
    );
}

struct Draft {
    id: String,
    name: String,
    expect: String,
    target: String,
    compare: Compare,
    why: String,
    line: usize,
}

fn emit(out: &mut Imported, levers: &mut Vec<Lever>, pending: &mut Vec<Leftover>, d: Draft) {
    let taken: Vec<Lever> = std::mem::take(levers);
    let unparsed: Vec<Leftover> = std::mem::take(pending);

    if !unparsed.is_empty() {
        for mut l in unparsed {
            l.reason = format!("{} (case {})", l.reason, d.id);
            out.leftovers.push(l);
        }
        return;
    }
    if taken.is_empty() {
        out.leftovers.push(Leftover {
            line: d.line,
            snippet: format!("faellt_mit \"{}\"", d.name),
            reason: format!(
                "case {}: no lever at all — it would build an unchanged tree and report green",
                d.id
            ),
        });
        return;
    }

    let expect = match rescue_pattern(&d.expect, d.compare) {
        Ok(p) => p,
        Err(e) => {
            out.leftovers.push(Leftover {
                line: d.line,
                snippet: d.expect.clone(),
                reason: format!("case {}: `expect` does not compile: {e}", d.id),
            });
            return;
        }
    };

    // An id is a key, so it must exist and must be unique — but a made-up
    // one is still made up, and the count says so at the end.
    let mut id = d.id;
    if id.is_empty() {
        id = slugify(&d.name);
        out.synthetic_ids += 1;
    }
    if out.cases.iter().any(|c| c.id == id) {
        let mut n = 2;
        while out.cases.iter().any(|c| c.id == format!("{id}-{n}")) {
            n += 1;
        }
        id = format!("{id}-{n}");
    }

    out.cases.push(Case {
        id,
        name: d.name,
        target: d.target,
        expect,
        compare: d.compare,
        why: d.why,
        levers: taken,
    });
}

/// grep's BRE reads `{`, `}`, `(` and `)` literally; this engine does not.
/// A pattern that fails to compile gets its braces escaped and is tried
/// once more — that is the faithful port, not a guess. Anything still
/// broken is named rather than shipped.
fn rescue_pattern(expect: &str, compare: Compare) -> Result<String, String> {
    if compare == Compare::Literal {
        return Ok(expect.to_string());
    }
    if compiles(expect) {
        return Ok(expect.to_string());
    }
    let escaped = escape_braces(expect);
    if compiles(&escaped) {
        return Ok(escaped);
    }
    Err(compile_error(expect))
}

fn compiles(p: &str) -> bool {
    regex::RegexBuilder::new(p)
        .case_insensitive(true)
        .build()
        .is_ok()
}

fn compile_error(p: &str) -> String {
    match regex::RegexBuilder::new(p).case_insensitive(true).build() {
        Err(e) => e
            .to_string()
            .lines()
            .last()
            .unwrap_or("")
            .trim()
            .to_string(),
        Ok(_) => String::new(),
    }
}

fn escape_braces(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 4);
    let mut escaped = false;
    for c in p.chars() {
        if escaped {
            out.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                out.push(c);
                escaped = true;
            }
            '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// The comment block above a case: `# 9c. <text>` gives the id, the rest
/// becomes `why`. Both are consumed.
fn head_and_why(comment: &mut Vec<String>) -> (String, String) {
    let text = comment.join("\n");
    let id = comment.first().map(|l| case_number(l)).unwrap_or_default();
    comment.clear();
    (id, text)
}

/// The case number at the head of a comment block — if there is one.
///
/// The numbering grew over a year of parallel sessions and is far more
/// varied than it looks: `9b.`, `198b.`, `9k2.`, `H6-1.`, `76-79.`. So the
/// token is any run of letters, digits and dashes that contains at least one
/// digit and ends in a full stop.
///
/// **And a good half of the cases carry no number at all** — measured on the
/// real script: 257 of 505 comment blocks start straight into prose. That is
/// why the caller has to be able to make an id up, and why it counts how
/// often it did. `fallkoepfe-pruefen.sh` only ever checked the numbers that
/// exist for duplicates; it never noticed how many are missing.
fn case_number(line: &str) -> String {
    let token: String = line
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if token.is_empty()
        || !token.chars().any(|c| c.is_ascii_digit())
        || !line[token.len()..].starts_with('.')
    {
        return String::new();
    }
    token
}

/// A readable id for a case whose comment block never gave one.
fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() { "ohne-namen".into() } else { s }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Write the cases back out as TOML.
///
/// Every string is a basic string with escapes, never a literal one. A
/// prettier emitter would have to decide per value whether `'…'` is safe,
/// and a sed expression is exactly the kind of value that makes that
/// decision wrong — it carries quotes, backslashes and pipes. Ugly and
/// always valid beats pretty and sometimes not; the round trip through
/// `cases::load_dir` is what proves it.
pub fn to_toml(cases: &[Case]) -> String {
    let mut out = String::new();
    out.push_str(
        "# Erzeugt von `tamper import` aus scripts/pruefungen-testen.sh.\n\
         # Ab hier ist ein Pruefall ein EINTRAG, kein Handgriff im Skript.\n",
    );
    for c in cases {
        out.push_str("\n[[case]]\n");
        out.push_str(&format!("id = {}\n", basic(&c.id)));
        out.push_str(&format!("name = {}\n", basic(&c.name)));
        out.push_str(&format!("target = {}\n", basic(&c.target)));
        out.push_str(&format!("expect = {}\n", basic(&c.expect)));
        if c.compare == Compare::Literal {
            out.push_str("compare = \"literal\"\n");
        }
        out.push_str(&format!("why = {}\n", multiline(&c.why)));
        out.push_str("levers = [\n");
        for l in &c.levers {
            match l {
                Lever::Sed { file, sed } => out.push_str(&format!(
                    "  {{ file = {}, sed = {} }},\n",
                    basic(file),
                    basic(sed)
                )),
                Lever::Perl { file, perl } => out.push_str(&format!(
                    "  {{ file = {}, perl = {} }},\n",
                    basic(file),
                    basic(perl)
                )),
                Lever::Script { script, files } => {
                    let list: Vec<String> = files.iter().map(|f| basic(f)).collect();
                    out.push_str(&format!(
                        "  {{ script = {}, files = [{}] }},\n",
                        basic(script),
                        list.join(", ")
                    ));
                }
            }
        }
        out.push_str("]\n");
    }
    out
}

fn basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    out.push_str(&escape(s));
    out.push('"');
    out
}

/// A multi-line basic string keeps its real line breaks — escaping them
/// would put the whole reason back on one line, which is the opposite of
/// what `why` is for. Every quote is escaped, so the text can never close
/// the string early.
fn multiline(s: &str) -> String {
    let mut body = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => body.push_str("\\\\"),
            '"' => body.push_str("\\\""),
            '\n' | '\t' => body.push(c),
            '\r' => {}
            c if (c as u32) < 0x20 => body.push_str(&format!("\\u{:04X}", c as u32)),
            c => body.push(c),
        }
    }
    format!("\"\"\"\n{body}\n\"\"\"")
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Split a shell line into argv, honouring single quotes, double quotes and
/// backslash escapes. Returns Err on unbalanced quoting, which the caller
/// turns into a named leftover rather than a guess.
pub fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut argv = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if has {
                    argv.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            '\'' => {
                has = true;
                let mut closed = false;
                for q in chars.by_ref() {
                    if q == '\'' {
                        closed = true;
                        break;
                    }
                    cur.push(q);
                }
                if !closed {
                    return Err("unbalanced single quote".into());
                }
            }
            '"' => {
                has = true;
                let mut closed = false;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                            None => return Err("line ends in a backslash".into()),
                        },
                        _ => cur.push(q),
                    }
                }
                if !closed {
                    return Err("unbalanced double quote".into());
                }
            }
            '\\' => match chars.next() {
                Some(n) => {
                    has = true;
                    cur.push(n);
                }
                None => return Err("line ends in a backslash".into()),
            },
            _ => {
                has = true;
                cur.push(c);
            }
        }
    }
    if has {
        argv.push(cur);
    }
    Ok(argv)
}
