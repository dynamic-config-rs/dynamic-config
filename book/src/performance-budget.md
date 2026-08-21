# The Performance Budget

The numbers this project holds itself to, and where each is measured.
A regression against any number here is treated like a failing test
whether or not a job catches it.

## The budget

| claim | budget | measured by |
|---|---|---|
| `current()` | ≤ baseline + 5% instructions | `benches/instructions.rs`, deterministic iai baselines committed beside it |
| allocations per read | **0** | `benches/alloc_profile.rs` |
| read under concurrent reload | scales with readers, no writer starvation | `benches/engine.rs` (read-under-fire), `benches/matrix.rs` (1→128 readers) |
| merge cost | linear in layers | `benches/matrix.rs` (2→16 layers) |
| reload end-to-end | dominated by parse+deserialize, not the swap | `benches/engine.rs` |
| a load, a reload, an `explain` | no worse than the release before, in instructions | `benches/instructions.rs`, whose header carries both releases' numbers |
| a million reloads | RSS bounded, FDs flat, no thread growth | the nightly `leak` job (`soak/`) |
| a five-hour fault schedule | every exit invariant holds | the nightly `soak` job; the 24-hour claim is a committed local run |
| compile time and binary size | monitored, not gated | the dependency-weight table in [MSRV & Features](msrv-features.md) |

## Against the neighbours

`benches-rivals/` races the same load against `config-rs` and
`figment` — a detached crate, so the rivals never touch this workspace's
dependency graph or its audits. Run it yourself:

```sh
cd benches-rivals && cargo bench
```

Cross-**language** numbers (Viper, Koanf, Dynaconf, pydantic-settings)
are architectural-cost positioning, not a race: a GC pause and an FFI
hop are not the same axis as a merge algorithm. Where the READMEs quote
them, they are hand-run, dated, and reproduced by the scripts they cite
— never CI-gated, because a shared runner cannot resolve the
differences that matter at this scale.

## The one rule

No number on this page may regress silently: a change that moves one
lands with the new measurement in the same commit, and the changelog
says which trade bought what.
