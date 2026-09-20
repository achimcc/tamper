# tamper

Mutation-test your build-time assertions: break the input on purpose and check
that the check goes red.

A configuration repository can be full of assertions that cannot fail. The only
way to know is to break the thing they guard and watch them. `tamper` does that
from a declarative list of cases — file, change, expected message — and keeps
apart the outcomes that a shell loop tends to merge.

```
tamper rules     # the seven verdicts, each explained
```

Two of the seven are not rulings at all: if the network was gone, or the build
never started, nothing was learned about the tree. Counting those as "all good"
is the failure this tool exists to prevent.

Work in progress. AGPL-3.0-only.
