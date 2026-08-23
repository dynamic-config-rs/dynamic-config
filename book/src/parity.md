# Parity Across Languages

One engine, three languages, and the honest table of what is the same,
what differs, and what is absent on purpose. This page doubles as the
specification the cross-language conformance fixtures check.

## Identical everywhere

| Behaviour | Rust | Python | Node |
|---|---|---|---|
| Precedence: files in order, then env, then flags/overrides | ✓ | ✓ | ✓ |
| Formats: JSON, TOML, YAML, INI, properties | ✓ | ✓ | ✓ |
| Env nesting `__`, prefix stripping, type widening | ✓ | ✓ | ✓ |
| `init` fails fast; a refused reload changes nothing | ✓ | ✓ | ✓ |
| Last-known-good cache, three modes | ✓ | ✓ | ✓ |
| Errors carry paths, never values | ✓ | ✓ | ✓ |
| `explain`/`check`, redacted by default | ✓ | ✓ | ✓ |
| Telemetry series (Prometheus text) | ✓ | ✓ | ✓ |
| Remote stores, same eight, same semantics | ✓ | ✓ | ✓ |
| `fingerprint()`: the same `sha256:…` for the same resolved tree | ✓ | ✓ | ✓ |
| `absent` as an error kind, told apart from `remote` | ✓ | ✓ | ✓ |

`fingerprint()` is the strongest row here, and the only one a fixture can
prove *across* languages rather than in each. The conformance case
`fingerprint-agrees-across-languages` pins one digest, and all three
runners compute it: a Rust process, a Python process and a Node process
that disagree about what configuration they are running would fail the
suite rather than a dashboard.

Its two properties travel with it. Declared secrets are masked by
position, so the digest moves when a secret appears or disappears and
stays put when one rotates — which is what makes it safe to log, label and
print in all three. And it is computed over the resolved tree rather than
any rendering, so TOML on one host and YAML on another fingerprint the
same.

## Same idea, different idiom

| Concern | Rust | Python | Node |
|---|---|---|---|
| Schema/validation | serde | dataclass / Pydantic / msgspec | Zod / Ajv / function |
| The read | `T::current()` → `Arc<T>` | `config.current()` → model | `config.current()` → object |
| Async waiting | `changes()` future | `changed_async()` / `events()` | `changes()` async iterator |
| Reload hooks | `on_reload` (watcher thread) | `on_reload` (watcher thread, GIL held — hooks must be quick) | `onReload` (event loop, unbounded queue) |
| Engine diagnostics | stderr / sink / `log` / `tracing` | `logging` (`dynamic_config.engine`), on by default | stderr, `setLogger` opt-in |
| Group reload | `ReloadGroup` (two-phase) | `ConfigGroup.reload_atomic()` | `group.reloadAtomic()` |

## Absent by design

| What | Where | Why |
|---|---|---|
| A `Wiring` lifecycle object | Rust | `main` already is one; the web crates' charter says no |
| Mounted health/metrics routes | Rust web crates | recipes in [their book](https://dynamic-config-rs.github.io/rust-web/production-surface.html) — composing engine surface beats freezing choices |
| Request scope for WebSockets | everywhere | a connection is not a request; both web books state the same rule |
| Free-threaded wheel for the remote package | Python | abi3-only today; the base wheel has one |
| A compiled addon per platform matrix beyond Tier 1 | Node | floors are stated in the README, extended on request |
| `run_until(shutdown)` | Python, Node | it takes a future and ends a watch when it resolves. Python's `asyncio` and Node's promises have their own shutdown idioms, and a translated one would be a third — `release()` and `AbortSignal` are what those languages already reach for |
| `WatchOptions` | Python, Node | the three numbers it names — debounce, ceiling, atomic-save grace — are reachable in Rust because a `Duration` is one type there. Both bindings take the debounce as a number and leave the other two at the engine's defaults, which is what every caller in either language has wanted so far |
| `RenewableSource`, `Revision`, `Lease` | Python, Node | trait objects a store crate implements. A binding surfaces the *documents* stores produce, not the seam they implement, and a lease renewed from Python would be a lease renewed off the thread that holds it |

A row moving from one table to another is a release-notes event, not an
edit: the conformance fixtures live beside the engine's test suite, the
bindings' CI resolves them, and a claim this page makes that a fixture
cannot check is marked with *(prose)* — today there are none.
