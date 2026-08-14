# Profiles & Discovery

## `name` + `paths`

```rust
DbConfig::builder("db")
    .discover("config", ["/etc/myapp", "~/.config/myapp", "."])
    .init()?;
```

`.discover(name, paths)` looks for `{name}.{ext}` in each directory, in
order. **Every** directory with a
match contributes one file, layered in search order — so `/etc` defaults,
`~/.config` overrides and a local `./config.toml` all apply, in that order.
(Stopping at the first hit would make naming both pointless: a machine-wide
file and a user's file are layers, not alternatives.)

Within one directory the extensions are tried `.toml`, `.json`, `.yaml`, `.yml`,
skipping any whose feature is off, and the first hit wins — so a stray
`config.json` next to a `config.toml` resolves the same way every run.

`~` expands via `HOME`, or `USERPROFILE` on Windows. Resolution happens per
load, so a file that appears later is picked up by the next reload rather than
requiring a restart.

Neither half works alone — a name without directories would search nowhere,
directories without a name would search for nothing — which is why
`.discover` takes both in one call rather than as two options that could be
stated separately.

## `profile_env`

```rust
DbConfig::builder("db").file("config.toml").profile_env("APP_ENV")
```

Names the variable holding the active profile. With `APP_ENV=production`, every
file gains a sibling layered over it: `config.toml`, then
`config.production.toml`. Works for discovered files too; a variant that does
not exist is skipped like any other missing file.

The profile is read at load time, so it follows the environment rather than the
build.

**A variant sits directly on top of its own base**, not above the search order:

```text
/etc/myapp/config.toml
/etc/myapp/config.production.toml
~/.config/myapp/config.toml            ← still wins over the line above it
~/.config/myapp/config.production.toml
```

So a later directory's plain file beats an earlier directory's variant. That is
the search order doing its job — a user's file is more specific to the machine
than a package's production defaults — but it is worth knowing before relying on
the opposite.
