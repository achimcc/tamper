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
tamper rules    the seven verdicts, each explained
```

## The seven verdicts

The point of the list is a distinction a shell loop tends to lose.

| verdict | means | build needed |
|---|---|---|
| `ok` | the build failed with the expected message | yes |
| `dead-lever` | the change altered nothing — the CASE is dead, not the check | no |
| `not-red` | the build was green: the check does not fire | yes |
| `other-message` | the build failed, but with a different message | yes |
| `broken-nix` | the edited file no longer parses | no |
| `network` | DNS or a download was gone — **no ruling** | — |
| `queue-timeout` | the build never started — **no ruling** | — |

**Two of them are not results.** If the network was gone, or the build never
started, nothing was learned about the tree, and counting that as "all good"
is the failure this tool exists to prevent. They get their own exit code.

**Two of them cost nothing.** A lever that no longer hits, and an edit that
destroys the syntax, are both decided before anything is built. The second one
matters more than it looks: when the file no longer parses, the *parser*
fails, long before any assertion runs — so the case proves that broken input
does not build, not that the check works.

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

`expect` is a case-insensitive **regular expression** by default, matched
against the build output with whitespace normalised — build tools wrap their
lines, and a pattern that spans the break would make a healthy check look
dead. `compare = "literal"` switches to a plain substring.

Levers are `sed`, `perl`, or `script` for anything else. `sed` and `perl` run
without a shell in between: the expression is one argv element, so it needs no
escaping of its own. A `script` lever must declare the `files` it touches,
because the cache key is computed before it runs.

## Exit codes

| | |
|---|---|
| 0 | every case ok |
| 1 | at least one finding |
| 2 | tamper could not run |
| 3 | the baseline is red — no ruling about any case |
| 4 | no finding, but cases without a ruling |

Exit 3 is the one worth explaining. Before the first case, `tamper` builds
each target **unsabotaged**. If the clean tree does not build, every case
would dutifully report "failed with the expected message" — a full set of
ticks for nothing. So a red baseline produces no verdicts at all.

## The cache

Re-running every case on every commit is the reason nobody runs them. The
result cache keys each ruling by the case itself, the files it edits, and the
files that define the checks. Everything else in the tree is assumed unable to
flip a ruling — a **named assumption**, documented with the counter-example
that breaks it, and held in place by three guards: only `ok` is stored, the
report names the cache's *coverage* and the age of its oldest entry rather
than a hit count, and `--no-cache` exists for a periodic full run.

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
