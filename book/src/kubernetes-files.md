# ConfigMaps, Secrets, and the Kubelet's Symlinks

How Kubernetes actually delivers a mounted ConfigMap or Secret to your
filesystem, and why this engine's file layers were built for that shape.
The [k8s book](https://dynamic-config-rs.github.io/k8s/) covers the
injector; what follows is the simpler, excellent pattern of **mounting
and watching** — no webhook required.

## What the kubelet really writes

A mounted ConfigMap is not files — it is symlinks:

```text
/etc/config/
  settings.toml -> ..data/settings.toml
  ..data        -> ..2026_08_19_00_00_00.1234567890/
  ..2026_08_19_00_00_00.1234567890/
    settings.toml
```

An update writes a NEW timestamped directory, then swaps the `..data`
symlink **atomically**. Consumers never see a half-written file — they
see the old tree, then the new tree.

## What that means for a watcher

- The file's path never changes; its *target* does. This engine's
  watcher handles the swap: the debounced reload re-reads through the
  path and gets the new target.
- There is a propagation delay (kubelet sync period, up to a minute by
  default) between `kubectl apply` and the swap. Watch latency is that
  delay plus your debounce, not just your debounce.
- A ConfigMap mounted with `subPath` does **not** update — the kubelet
  binds the file once. Never use `subPath` for configuration you want
  live.

## Secrets directories

A mounted Secret has the same symlink shape, one file per key — which
is exactly what `secrets_dir` reads: the filename is the key, `..data`
is skipped as a directory, the per-key links are followed. Since 0.8.0
a link may not resolve *outside* the mount ([the containment
contract](secret-lifecycle.md)); the kubelet's links resolve inside by
construction and are untouched by that rule.

## The minimal deployment

```yaml
volumeMounts:
  - { name: config, mountPath: /etc/config, readOnly: true }
volumes:
  - configMap: { name: app-config }
    name: config
```

```rust
AppConfig::builder("app")
    .file("/etc/config/settings.toml")
    .init()?;
AppConfig::builder("app")
    .file("/etc/config/settings.toml")
    .watch(Duration::from_millis(500))?
    .detach();
```

Readiness through it all: [one page](readiness.md), already written.
