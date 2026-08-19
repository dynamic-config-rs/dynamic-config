# Readiness and Liveness

What `/livez` and `/readyz` should say about configuration, written
once. The config server, the Python web adapters and the Kubernetes
book link here.

## The positions

- **Liveness**: configuration says nothing about it. A process with a
  broken config file is alive; killing it cannot fix the file, and a
  restart loop turns one bad document into an outage.
- **Readiness before the first install**: not ready. A service that
  never loaded configuration has nothing correct to serve. `init()`
  failing at startup *should* fail startup — last-known-good exists for
  processes already running, not to launch new ones against stale data
  (unless you opted into a `cache`, which is exactly that decision,
  made explicitly).
- **Readiness after the first install**: **ready while last-known-good
  serves — including through refused reloads and a down remote store.**
  Flipping unready because the store blinked converts a degraded
  control plane into a data-plane outage: Kubernetes drains the very
  pods that are still serving correctly from their last good snapshot.
- **Degradation is a detail, not a state**: expose it — the health
  payload carries `consecutive_failures`, `snapshot_age_seconds`,
  `source_up` — and alert on it. A human decides whether a
  three-hour-stale config warrants draining; a probe should not.

## The shape

```json
{ "status": "ok", "degraded": true,
  "config": { "generation": 812, "consecutive_failures": 4,
              "snapshot_age_seconds": 947, "source_up": false } }
```

HTTP 200 with `degraded: true` — ready, loudly imperfect. The one
exception a deployment may choose: an explicit staleness ceiling
(`snapshot_age_seconds` beyond N → unready), which is a business
decision this page cannot make for you; state it in your runbook if
you take it.
