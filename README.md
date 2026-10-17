# tamper

Mutation-test your build-time assertions: break the input on purpose and check
that the check goes red.

A configuration repository fills up with assertions — this guest must order
itself after its device node, that bind mount must not carry `:idmap`, this
service must not keep its factory password. Every one of them is written once
and then trusted for years. Some of them cannot fail at all: they check the
source they were derived from, or their pattern finds itself, or the thing
they guard was renamed underneath them. A check that cannot go red is
indistinguishable from a check that is satisfied.

`tamper` takes a declarative list of cases — file, change, expected message —
breaks each one in a throwaway worktree, builds, and judges what came back.

```
tamper run      break each case's file, build, judge the message
tamper dry      does each lever still hit? (no build, seconds)
tamper list     the cases, their targets, their soundness
tamper rules    the eight verdicts, each explained
```

## The eight verdicts

The point of the list is a distinction a shell loop tends to lose.

| verdict | means | build needed |
|---|---|---|
| `ok` | the build failed with the expected message | yes |
| `dead-lever` | the change altered nothing — the CASE is dead, not the check | no |
| `not-red` | the build was green: the check does not fire | yes |
| `false-alarm` | the build failed, although the case says it must stay green | yes |
| `other-message` | the build failed, but with a different message | yes |
| `broken-nix` | the edited file no longer parses, and the build failed with another message | yes |
| `network` | DNS or a download was gone, and the expected message is not there — **no ruling** | — |
| `queue-timeout` | the build never started — **no ruling** | — |

**One of them reads the other way round.** Most cases break something and
expect the check to go red. A few say the opposite: this change is legitimate,
and the check must stay GREEN. `green = true` on the case states that, and
`false-alarm` is what comes back when the check fires anyway — a check that is
too eager is as broken as one that never fires, and calling that `not-red`
would say the reverse of what happened.

**Two of them are not results.** If the network was gone, or the build never
started, nothing was learned about the tree, and counting that as "all good"
is the failure this tool exists to prevent. They get their own exit code.
One exception, and it came from a real run: if the output carries the message
the case is waiting for, evaluation got as far as the assertion, and that is
a ruling — whatever the network patterns say. A check whose own text *quotes*
a DNS error (`Could not resolve hostname`) lost its ruling that way.

**One of them costs nothing.** A lever that no longer hits is decided before
anything is built.

**`broken-nix` needs the build to be sure.** When the edited file no longer
parses, the *parser* usually fails long before any assertion runs — so the
case proves that broken input does not build, not that the check works. But
not every check parses what it reads: one that scans files as text fires on a
broken file just the same. So the syntax error is found first and turned into
a verdict only if the build then fails with a *different* message. `tamper
dry` cannot build, and says so next to every syntax error it reports.

## Cases

```toml
[[case]]
id = "9c"
name = "bind mount with :idmap over a mount of its own"
target = "server"
expect = "over a mount of its own"
why = """
The relapse: /tank/photos with :idmap again, although tank/photos/<name> are
datasets of their own. It stood like that in the repo for nine hours — the
running guest never noticed, it only stopped STARTING, and nothing went red.
"""
levers = [
  { file = "lib/guests.nix", sed = "s|a|b|" },
]
```

A case says what it expects in one of three ways: the message the build must
carry (`expect`), the checks it must bring down (`checks`), or that the check
must stay GREEN (`green = true`). `expect` and `checks` may stand together —
then both must hold. `green` next to either, or none of the three, is refused
at load time.

`expect` is a case-insensitive **regular expression** by default, matched
against the build output with whitespace normalised — build tools wrap their
lines, and a pattern that spans the break would make a healthy check look
dead. `compare = "literal"` switches to a plain substring.

### Naming the check instead of its message

```toml
[[case]]
id = "103"
name = "guest without snapshot coverage"
target = "server"
checks = ["P-103"]
why = "..."
levers = [ { file = "lib/guests.nix", sed = "s|a|b|" } ]
```

A pattern is a guess about the text; an id is the check itself. The case
above is `ok` only if the build output carries the marker `[P-103]` —
literally, case-sensitively, after the same whitespace normalisation as
`expect`. Every id in the list must have left its marker. `expect = "check
103"` also matched `check 103b`, and a case whose lever brought down the
wrong check came back `ok`; `[P-103]` does not match `[P-103b]`.

An id is `P-` followed by lowercase letters, digits and hyphens
(`^P-[a-z0-9][a-z0-9-]*$`): `P-103`, `P-ryot-promises`. The checks on the
other side start their message with that marker, `[P-103] …`, and nothing
else about them changes. A build that went red without one of the markers is
`other-message` — the same ruling as a different message, and the detail
names the markers that are missing. Set `expect` next to `checks` when the
wording matters too.

Levers are `sed`, `perl`, or `script` for anything else. `sed` and `perl` run
without a shell in between: the expression is one argv element, so it needs no
escaping of its own. A `script` lever must declare the `files` it touches,
because the cache key is computed before it runs.

"Without a shell" is about quoting, not trust, so two fences keep a lever in
its throwaway tree: every `file`/`files` path must be relative and free of
`..` (checked when the cases load, so `tamper list` refuses it), and a `sed`
or `perl` lever's file must *resolve* inside the tree — a symlink pointing out
is refused too. `sed` runs with `--sandbox`, which rejects the `e`, `r` and `w`
commands. `perl` expressions and `script` levers remain code from the case
file, trusted like the repository that holds it.

An interrupted run (SIGINT, SIGTERM, SIGHUP) removes its own worktrees before
it exits with 128 + signal — and prints what it had ruled until then, so a CI
job that hits its time limit does not end without a word. The trees of a run
that was killed outright are removed at the next start, once its process is
gone.

While it runs, `tamper` writes one line per ruling to stderr.

## Exit codes

| | |
|---|---|
| 0 | every case ok |
| 1 | at least one finding |
| 2 | tamper could not run |
| 3 | a target's baseline is red — its cases were not run, the others were |
| 4 | no finding, but cases without a ruling |
| 5 | no finding, but the ledger's oldest entry is older than `--max-age` |

Exit 3 is the one worth explaining. Before the first case of a target,
`tamper` builds that target **unsabotaged**. If the clean tree does not build, every case
would dutifully report "failed with the expected message" — a full set of
ticks for nothing. So a red baseline produces no verdicts for its target: the
report names the target, why its clean tree is red, and how many cases were
not run. The other targets run as usual — one upstream hash mismatch should
not leave five hundred cases on healthy targets unjudged. A finding still
outranks exit 3; exit 3 outranks exit 5, and both outrank cases without a
ruling.

## The cache

Re-running every case on every commit is the reason nobody runs them. The
result cache keys each ruling by the case itself, the flake attribute its
target builds, the files it edits, and the files that define the checks — all read from the commit under test
(`--commit`, default `HEAD`), never from the working tree, because the commit
is what gets built. Everything else in the tree is assumed unable to
flip a ruling — a **named assumption**, documented with the counter-example
that breaks it, and held in place by three guards: only `ok` is stored, the
report names the cache's *coverage* and the age of its oldest entry rather
than a hit count, and `--no-cache` exists for a periodic full run.

The cache is an optimisation and never decides a case. If a key cannot be
computed — a declared definition file is missing from the commit under test —
the case is built without the cache and the report says why. Until 0.5.1 that
case came back `not-red` without any build: "the check does not fire",
measured on nothing.

## The ledger

A periodic full run is a promise only while it finishes. Every case is a
build, and an inventory grows: the run this tool was written for went from
116 cases to 789, each a full NixOS evaluation, and was cut off by its time
limit every week — eight runs, none green, and since the report came at the
end, nothing left behind. "A stale `ok` lives at most seven days" had never
been true, and nothing said so.

The ledger is the answer to a run that cannot get through everything in one
go. It keeps one small file per case — verdict, time, commit — written the
moment the case is ruled, independent of the tree (the cache cannot do this:
one commit to the check definitions turns every cache key at once).

```
tamper run --no-cache -j 1 --oldest-first --budget 170m --max-age 21 --metrics tamper.prom
```

- `--oldest-first` rules what has never been ruled first, then the standing
  findings, then `ok` from the oldest ruling to the youngest.
- `--budget` starts no new case once the time is up. The case under way is
  finished; the rest keeps its place in line for the next run.
- A **finding stands** until its case is ruled again: a run that did not get
  to it still reports it and still exits 1. Standing findings go second in
  line, so a repaired case turns green the next night.
- `--max-age` turns the promise into a check: exit 5 when the oldest entry is
  older than that. A case that was never ruled is measured from the day it
  was first seen — new cases are not overdue on their first day, and none can
  wait unnoticed.
- `--metrics` writes the ledger's state as a Prometheus text file
  (`tamper_oldest_ruling_seconds`, `tamper_never_ruled`,
  `tamper_standing_findings`, …), so the age of the oldest ruling can be
  watched instead of believed.

The report names the gap, as the cache's does: how many cases this run ruled,
how many it did not reach, how many were never ruled, and which entry is the
oldest. `network` and `queue-timeout` are not rulings and leave an entry
untouched. The entries of deleted cases are dropped at the start of a run.

The ledger lives in `$XDG_STATE_HOME/tamper/<repository>` unless `--ledger`
says otherwise — state, not cache: losing it makes every case look new.
Without one of these switches a run keeps no ledger and behaves as before.

## Install

```
nix run github:achimcc/tamper -- rules
```

Or add the flake as an input and take `packages.default`.

## Running the tests

`cargo test` skips four tests that drive `nix-instantiate`, and says so. The
full run is `cargo test -- --include-ignored`; `nix flake check` covers
everything else, including clippy and rustfmt.

## License

AGPL-3.0-only.
