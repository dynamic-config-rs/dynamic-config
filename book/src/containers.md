# Containers: Docker, Compose, Podman

No orchestrator — one container, one bind-mounted config, hot reload
intact. The [Kubernetes book](https://dynamic-config-rs.github.io/k8s/)
owns the cluster story; what follows is `docker run`, the two composes,
and the three caveats that actually bite.

## The three caveats, first

1. **Bind-mount file events are a platform property.** On a Linux host,
   editing a bind-mounted file reaches the container's inotify — same
   kernel, events flow, the default watcher just works. On **Docker
   Desktop** (macOS/Windows) the mount crosses a VM boundary and events
   are lossy or absent — and silently so: the watch registers and never
   delivers. Choose the watcher's **poll mode** there
   (`watch_with(…, WatchMode::Poll { interval })` in Rust, `pollMs` in
   Node, `poll_interval` in Python) — the same explicit knob
   [Hot Reload & Watching](hot-reload.md) prescribes for NFS and
   overlay filesystems.
2. **Editors rename, and the rename must land whole.** The engine's
   atomic-save grace covers it; nothing to do — stated so you do not
   chase it.
3. **Podman + SELinux relabels or refuses.** Mount with `:Z` (private)
   or `:z` (shared) on Fedora-family hosts, or the container reads
   nothing and the error blames permissions.

## `docker run`

```sh
docker run --rm \
  -v ./config:/etc/myapp:ro \
  -e MYAPP_DB__POOL_SIZE=32 \
  myapp:1
```

The app inside declares both layers once — file under env, exactly as
everywhere else:

```rust,ignore
AppConfig::builder("app")
    .file("/etc/myapp/config.toml")
    .env("MYAPP_")
    .init()?;
AppConfig::builder("app")
    .file("/etc/myapp/config.toml")
    .env("MYAPP_")
    .watch(Duration::from_millis(500))?   // WatchMode::Poll on Docker Desktop
    .detach();
```

Edit `./config/config.toml` on the host; the container reloads. The env
layer stays start-frozen — that is the container runtime's rule, and
[the k8s book's env-inject page](https://dynamic-config-rs.github.io/k8s/annotations.html)
tells the same truth with a restart trigger attached.

## docker-compose

```yaml
services:
  app:
    image: myapp:1
    volumes:
      - ./config:/etc/myapp:ro
    environment:
      MYAPP_DB__POOL_SIZE: "32"
    healthcheck:
      # /readyz per the readiness contract: LKG serving = ready.
      test: ["CMD", "wget", "-qO-", "http://127.0.0.1:8080/readyz"]
      interval: 15s

  # Optional: the config server in front of the stores, so `app` holds
  # no store credentials — the same indirection the k8s book uses. The
  # remote book ships a complete compose for it:
  # https://github.com/dynamic-config-rs/dynamic-config-remote/tree/main/examples/compose
```

`docker compose restart app` is the env-refresh story; file edits need
nothing.

## Podman, and podman-compose

Same flags, two differences worth their line:

```sh
podman run --rm \
  -v ./config:/etc/myapp:ro,Z \
  -e MYAPP_DB__POOL_SIZE=32 \
  myapp:1
```

- **`,Z`** — the SELinux label from caveat 3.
- **Rootless UID mapping**: files created by the container land on the
  host under a mapped UID; a `secrets_dir` mounted in must be readable
  by the container's mapped user — `podman unshare chown` is the tool
  when it is not.

`podman-compose` consumes the compose file above unchanged. One honest
boundary: **`podman kube play` runs pods, not admission webhooks** — the
k8s book's injector does nothing there. Use this chapter's shape (the
file pattern, straight mounts) for podman-managed pods, and keep the
injector for real clusters.
