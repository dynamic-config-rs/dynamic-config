# How Resolution Works

Every question this crate answers about a value — what it is, where it came
from, why it beat the other thing — is answered by one walk, described here
in order. Two parts of the compatibility contract rest on it:
[§2, precedence never changes silently](compatibility.md), and
[§4, provenance stays stable](compatibility.md).

## One contribution per layer

A load names a section — `db`, say — and every configured source is asked
the same question: *what do you have to say about this section?* The answer
is a **contribution**: a tree of values, plus the origin to record for them.

```text
defaults        {port: 5432}                    runtime "default"
config.toml     {host: "db.internal"}           file  config.toml
APP_DB_*        {port: 6543}                    env   APP_DB_*
--set           {pool: {max: 32}}               runtime "command-line flag"
```

A source that says nothing about this section contributes nothing. A source
that is not configured is not asked. A file that is not there is not an
error: every file layer is optional, and a deployment that ships two of
three documents is ordinary.

The order the contributions are collected in **is** the precedence order:

| | layer | why it sits here |
|---|---|---|
| lowest | defaults | anything at all displaces them |
| | discovered files | a search result is a guess about the machine |
| | listed files | `files = [..]` names a file on purpose |
| | remote | a central store beats what a package shipped |
| | secrets directory | a mounted secret is a fact about *this* deployment |
| | `.env` files | the environment layer, sourced from disk |
| | environment | a variable exported for this run beats a file |
| | bindings | wiring, not whatever the deployment happens to export |
| | flags | typed by a person for this one run |
| highest | overrides | nothing displaces them, which is what makes a test authoritative |

## The fold

The contributions go to the **[engine](engines.md)**, which merges them
lowest to highest by one rule, applied at every depth:

> **Tables descend. Everything else replaces.**

Two tables at the same key are merged key by key. Anything that is not a
table — a scalar, a list, a null — replaces what was under it whole. **A
list is not appended to**: `tags = ["a"]` in a higher layer is the tags, not
one more tag. This is the rule the whole crate is built on, and it is the
same rule at the top level and ten keys deep.

Which engine folds them is a choice — the `config` crate's, figment's, or
this crate's own — and it is a choice about whose code runs rather than
about what a configuration means: they implement the same rule, and the
tests compare them leaf by leaf.

While the fold runs, the origin of each leaf is written down as that leaf is
won. Nothing afterwards has to work out where a value came from by
inspecting what supplied it — the answer was recorded at the moment it
became true. That is what [`explain()`](validation-diagnostics.md) and
`source_of()` read.

One narrowing happens after the fold: an environment origin starts as the
prefix it was matched by (`APP_DB_*`) and is narrowed to the exact variable
(`APP_DB_POOL__MAX`) for leaves the environment layer actually supplied —
derived from the path and then *checked* against the real environment, so a
variable nobody set is never named.

## Aliases, then the snapshot

[Aliases](migrations/from-0.6.md) run as a pass over the folded tree rather
than as a layer, because that is what an alias is: a gap-fill for a key
somebody renamed. An alias only fills a destination that nothing above the
defaults supplied, and it can read a *sibling* section, which is why the
walk keeps every section's contributions and not only the one being loaded.

What comes out is a snapshot: the resolved tree, plus the origin of every
leaf. Reading it into your type is the last step and touches no source at
all — which is why `check()` can report on every key for the cost of one
walk rather than one walk per key.

## Where the backend went

Through 0.8 this walk was figment's: sections were its profiles, the merge
was its merge, the environment was its provider, and provenance was
recovered afterwards by recognising a provider from the words in its name.
All of that is now this crate's, with one part left swappable: the fold
itself, which is what an [engine](engines.md) is — and this crate keeps
no fold of its own, because two implementations of somebody else's rule
is one more than the rule needs. figment is one of the two that ship, and
also stays reachable as a *source* through
[`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider).

That is a claim about behaviour, so it is tested as one. figment stays a
permanent dev-dependency, and four differential tests hold the port to the
original: the value-string reader, over generated strings; the
deserializer, over generated trees into twenty-one target types, compared on
the value *and* on the path an error stopped at; the target types
themselves — thirty-three of them, from `bool` and `IpAddr` to all four
serde enum representations and the loose readings (`42` as a `String`,
`"8080"` as a `u16`), each read through both paths and asserted equal,
including the two both refuse: `i128` and `u128` past what a JSON
document can carry; and the fold itself, over the layer stacks a real load produces.
