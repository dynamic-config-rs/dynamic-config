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

A row moving from one table to another is a release-notes event, not an
edit: the conformance fixtures live beside the engine's test suite, the
bindings' CI resolves them, and a claim this page makes that a fixture
cannot check is marked with *(prose)* — today there are none.
