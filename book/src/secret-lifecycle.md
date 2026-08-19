# The Secret Lifecycle, Honestly

Where a secret lives from store to snapshot, how many copies exist,
when they die — and the boundary of what this project can promise. A
threat model that overclaims is worse than none.

## The path, copy by copy

```text
store (Vault, a mounted file, an env var)
  → transport buffer            dropped when the fetch returns
  → the document's text         one String, dropped after parse
  → the Value tree              one copy, dropped after deserialize
  → your typed struct           one copy, inside the Arc snapshot
  → Arc<T> snapshots            the ONLY long-lived copy
```

Old snapshots drop when the last reader releases them — an `Arc`
refcount, not a garbage collector's whim. A reader that clones a
snapshot into a long-lived structure extends that lifetime knowingly;
nothing else does.

## What rotation leaves behind

After a reload, the previous secret survives exactly as long as the
previous snapshot: until the last in-flight request holding it
completes. There is no cache of former values, no history — the
`cache` feature writes **redacted** documents by default, and refuses
redaction-dependent modes when it cannot tell which fields are secret.

## The zeroization boundary, stated plainly

**Full zeroization is not promised, because it is not promisable.**
Your configuration type is an arbitrary user struct behind an `Arc`;
its drop order, its allocator, its copies made by `Clone` derives are
yours, not this crate's. Freed memory holding stale secret bytes until
reuse is a property of the allocator. A process whose threat model
includes an attacker reading its freed heap needs OS-level answers
(locked memory, encrypted swap, short-lived processes) — a `Zeroize`
bolt-on here would promise what an `Arc<T>` it cannot see into does
not deliver.

## What IS promised, and enforced

- No configuration value — secret-marked or not — in any `Debug`,
  `Display`, error, diff, report, log line or metric label. Pinned by
  `tests/security.rs`, the `redaction` fuzz target, and (since 0.7.1)
  the serde-message scrub the `lkg_serves_previous` fuzz target forced.
- Cache files are `0600`, created with `create_new`, redacted by
  default.
- The secrets directory cannot be walked out of by a planted symlink
  (0.7.1; the [containment note](compatibility.md) and its adversarial
  suite).
- Remote stores authenticate over TLS whose verification cannot be
  turned off by any flag this family ships.
