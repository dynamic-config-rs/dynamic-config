# The Metrics Contract

The standard schema every exporter in this family emits or maps to.
Canonical here; the server, the web adapters and the Kubernetes agent
link rather than restate.

## The names

| metric | type | meaning |
|---|---|---|
| `dynamic_config_reload_total` | counter | installs since start |
| `dynamic_config_reload_failed_total` | counter | refusals since start (the monotonic `refusals()`, not the resetting streak) |
| `dynamic_config_last_success_timestamp_seconds` | gauge | when the serving snapshot landed |
| `dynamic_config_generation` | gauge | the published install counter |
| `dynamic_config_snapshot_age_seconds` | gauge | `stale_for()`, exported |
| `dynamic_config_consecutive_failures` | gauge | the resetting streak — the alerting number |
| `dynamic_config_source_up` | gauge | 1 when the remote source's last fetch succeeded |
| `dynamic_config_source_failure_total` | counter | remote fetch failures |
| `dynamic_config_watch_running` | gauge | 1 while a watcher thread is alive |

## The labels

Exactly two: `config` (the section key) and `source` (the store's
short name, where a remote is involved). Both are drawn from
**code-shaped identifiers with bounded cardinality** — a deployment has
a handful of configuration types and a handful of stores.

The hard rules, in the order an auditor asks:

1. **No configuration values.** Not even non-secret ones — today's
   hostname label is tomorrow's connection string.
2. **No user-supplied paths.** A key path can encode tenant names.
3. **No free text.** Failure *kinds* are a closed enum; messages never
   become labels.
4. **No unbounded label.** Anything per-request, per-user or
   per-document is a cardinality explosion wearing a metric's clothes.

## Alerting starters

`consecutive_failures > 0` for longer than one reload interval — the
document is broken and someone should read the log line that says which
key. Rising `reload_failed_total` with flat `reload_total` — every
attempt refused; the previous snapshot is serving and ageing, watch
`snapshot_age_seconds` against your staleness tolerance.
