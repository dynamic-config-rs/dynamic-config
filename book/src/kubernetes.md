# Kubernetes

The Kubernetes integration has **a book of its own**:

### → [dynamic-config on Kubernetes](https://dynamic-config-rs.github.io/k8s/)

```sh
helm install dynamic-config \
  oci://ghcr.io/dynamic-config-rs/charts/dynamic-config
```

```yaml
metadata:
  annotations:
    dynamic-config.rs/inject: "true"
    dynamic-config.rs/source: "vault"
    dynamic-config.rs/endpoint: "https://vault.vault.svc:8200"
    dynamic-config.rs/key: "secret/myapp"
    dynamic-config.rs/auth: "kubernetes"       # the pod's own identity
    dynamic-config.rs/path: "/config/rendered.yaml"
```

Annotate a pod and a webhook injects a rendering agent: the store's
document arrives as a **live file** (this engine's watcher picks it up —
[Kubernetes Files](kubernetes-files.md) is the file-shape contract),
as **real environment variables** (`env-inject`, with an opt-in
container-restart on change), or as a **native Kubernetes Secret** the
operator keeps reconciled for consumers that read nothing else.

What the k8s book carries: the annotation contract, all nine stores
with every auth method (identity-first: the pod's service account
wherever the store speaks it), three TLS modes for the webhook
including one that rotates itself, an operator with namespaced AND
cluster-scoped store classes, honest comparison tables against the
Vault Agent Injector and External Secrets Operator, and twenty-six
ready-to-apply examples — six of them on real software (Airflow,
Grafana, Kafka, Postgres, a four-component shop stack).

The same rule as every family member: the agent resolves through THIS
engine, so precedence, validation, provenance and redaction behave
byte-for-byte like the crate this book documents.
