# The Rest of the Family

This book is the engine: what resolves a configuration, what stores it,
what watches it, and the macro that declares one. Four crates.

Everything else that carries the name lives in a repository of its own,
with a book on this same site. They are separate because they release
separately — a store's client library moving, or a Node package needing a
fix, is not a reason to re-cut the engine — and each of them names this
engine with a caret, so it picks up a patch release here when it is ready
rather than when it is forced.

| Book | What it covers | Ships as |
|---|---|---|
| [Remote stores](https://dynamic-config-rs.github.io/remote/) | etcd, Consul, Vault, NATS, Redis, S3, Firestore and git — and the server that hands a section to a client that cannot reach any of them | eight crates, plus `dynamic-config-server` |
| [Python](https://dynamic-config-rs.github.io/python/) | the same engine behind a Python API: dataclasses, Pydantic, msgspec, asyncio | `dynamic-config-py` on PyPI |
| [Node.js](https://dynamic-config-rs.github.io/node/) | the same engine behind Node-API: Zod, Ajv, a plain function, a watcher that never blocks the loop | `dynamic-config-node` on npm |

**What crosses between them is the trait.** A store implements
[`RemoteSource`](https://docs.rs/dynamic-config/latest/dynamic_config/trait.RemoteSource.html) — or its async twin — and the engine
knows nothing else about it. That is what lets the stores live elsewhere
without this crate having a feature flag for each of them, and what lets
somebody write a store this project has never heard of.
