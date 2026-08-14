# Credits

What this engine took from other people's work, and from whom. The
sentences below are the whole of it: everywhere else in this repository,
the prose is about *this* code, because a reader who has never used the
library being compared to is being asked to hold a comparison they did
not come for.

## Built on

**[figment](https://docs.rs/figment)** — the layering and merge engine
underneath every load. Providers, profiles, the `Value` tree and the
merge semantics are figment's; what this crate adds is the declaration,
the snapshot cell, the watcher, provenance that survives the merge, and
a set of refusals figment deliberately leaves to its caller. Where the
two disagree — reserved profile names, environment provenance — the
reasoning is in [the book](book/src/limitations.md), not here.

**[serde](https://serde.rs)** — the only bound this crate puts on a
configuration type is `DeserializeOwned`, which is why a plain struct, a
schemaless `Value` and somebody else's derive all work unchanged.

**[notify](https://docs.rs/notify)** — the filesystem watcher, and the
three platform backends that make "edit a file and watch the snapshot
follow" mean the same thing on Linux, macOS and Windows.

<!-- The repositories rather than pyo3.rs and maturin.rs: the doc sites
     refuse connections from CI runners often enough to have taken a
     release's link check down, and a link that fails a gate for somebody
     else's firewall is a link that gets excluded and then rots. Both
     repositories link to their books from the first line of the README. -->
**[PyO3](https://github.com/PyO3/pyo3)** and
**[maturin](https://github.com/PyO3/maturin)** — the
Python binding and its wheels; **[napi-rs](https://napi.rs)** — the Node
binding and its prebuilt binaries.

## Ideas taken

**Go's [Viper](https://github.com/spf13/viper)** — the shape of the
problem. Reading one configuration from files, an environment and a
remote store, with a search path and a watcher, is Viper's design, and
the crate's discovery, `Snapshot::sub` and `save_new` all have Viper
ancestors. Three of its decisions were deliberately not taken —
lowercasing every key, a global default instance, and type-by-default —
each for a reason written out in
[Limitations](book/src/limitations.md).

**[Spring Cloud Config Server](https://spring.io/projects/spring-cloud-config)**
— the URL shape `dynamic-config-server` serves: application, then
profile, then the document. An operator who has run one already knows
what `GET /billing/prod` returns, and that transfer was worth more than a
scheme of our own.

**[iai-callgrind](https://github.com/iai-callgrind/iai-callgrind)** — the
argument that a benchmark which cannot fail is not a gate, and the
machinery that lets instruction counts be one.

## Comparisons

[Comparisons](book/src/comparisons.md) sets this crate beside `config`,
`figment` and Viper feature by feature, and
[Limitations](book/src/limitations.md) collects what it will not do and
why. Both are about the differences; this page is about the debt.
