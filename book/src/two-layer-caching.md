# Two-Layer Caching, as a Use Case

The engine is, structurally, a two-layer configuration cache — and
leaned on as one, that is a deployment architecture of its own:

```text
L0   the Arc snapshot        every read; a pointer load, never I/O
L1   the disk cache          cache(path, mode) — written after every
                             clean install, recovered at boot
L2   the store               file, Vault, etcd, S3 … — touched only
                             by init, reload and the watcher
```

The property that makes it a *cache* rather than a copy: **reads never
fall through.** `current()` answers from L0 unconditionally; L1 exists
for the next process start; L2 is consulted on the refresh schedule you
chose and at no other time. Store latency, store outages and store rate
limits are refresh-path concerns — the read path cannot observe them.

## What each layer buys

- **L0 — outage survival while running.** A store that goes away mid-run
  costs nothing: last-known-good keeps serving, `status()` says the
  refreshes are failing, and [readiness stays ready](readiness.md).
- **L1 — outage survival across restarts.** The reboot case: the store
  (or the mount, or the network) is not up yet, and a service that
  would otherwise sit dead comes up on yesterday's configuration —
  loudly, with a warning per recovery, because quietly running stale is
  its own outage:

  ```rust,ignore
  AppConfig::set_remote(store);
  AppConfig::refresh_remote().ok();  // best-effort: L1 covers the miss
  AppConfig::builder("app")
      .cache("/var/cache/myapp/last.json", CacheMode::Redacted)
      .init()?;                       // sources broken → recovered from L1
  ```

- **L2 — one source of truth.** The fleet converges on the store's
  document at watch cadence; nothing needs a rollout.

## The honest boundaries

This is two-layer caching **for configuration**, and it says so:

- **One document per key, latest wins.** There is no per-entry TTL, no
  eviction, no read-through-per-key — a general-purpose KV cache
  (your Redis-in-front-of-Postgres) is a different tool and this crate
  does not want the job.
- **Secrets shape the disk layer.** `Redacted` (the default) drops
  `#[config(secret)]` fields on disk, which means recovery only works
  if the secrets come from somewhere live; `Full` keeps everything and
  says so out loud; `cache_encrypted` is full fidelity at rest through
  your `Encryptor`. The [Secret Lifecycle](secret-lifecycle.md) page
  carries the copy-count story.
- **Recovery is loud and opt-in.** No cache exists unless you asked;
  every recovery logs a warning; a fresh install rewrites L1 so the
  stale window is one outage long, never cumulative.

## When to reach for it

A service whose store is remote and whose restart must not depend on
that store being reachable — spot instances, edge nodes, anything that
reboots into a network it does not control. The
[k8s agent](https://dynamic-config-rs.github.io/k8s/) is this same
architecture with the layers made of Kubernetes objects: store → agent
→ rendered file (L1) → your watcher (L0).
