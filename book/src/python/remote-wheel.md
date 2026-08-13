# Remote Stores in Rust, from Python

```sh
pip install dynamic-config-py[remote]
```

```python
from dynamic_config import DynamicConfig
from dynamic_config.remote import Etcd

config = DynamicConfig(Database, key="db").remote(
    Etcd(["http://etcd.internal:2379"], "myapp/db.json")
)
config.refresh_remote()
config.init()
```

The other half of [Remote Stores in Python](remote-stores.md). That page
is the door a Python `fetch()` goes through; this one is the Rust store
clients, compiled, behind an opt-in install.

**All eight stores are here** — Consul, etcd, Firestore, git, NATS, Redis,
S3 and Vault — each mirroring its Rust crate's builder.

| Store | Client | Runtime | The credential |
|---|---|---|---|
| `Consul` | `ureq`, blocking | none | An ACL token, or a bearer exchanged for one |
| `Etcd` | gRPC, async | tokio | A user and password, in the connection |
| `Firestore` | `ureq`, blocking | none | An access token, or the metadata server |
| `Git` | `gix`, blocking | none | An https token, or an ssh key |
| `Nats` | `async-nats` | tokio | A token, NKey, user pair or `.creds`, on connect |
| `Redis` | `redis`, blocking | none | A user and password, in the URL |
| `S3` | AWS SDK, async | tokio | A `ProvideCredentials` chain — see [below](#s3-the-credential-that-is-a-trait) |
| `Vault` | `ureq`, blocking | none | A login that buys a token with a TTL |

## Why it is a second wheel

A wheel is built per platform, so a dependency in the ordinary wheel is
in every install of it — including the ones reading a single TOML file.
etcd speaks gRPC, the AWS SDK brings its own runtime and signing stack,
NATS brings a protocol, git brings `gix` and three more bring HTTP over
rustls: eight clients is most of an async ecosystem, in every install
that wanted a `config.toml`.

An extra alone cannot do it. `pip install dynamic-config-py[etcd]`
installs *Python distributions*; it cannot turn on a Cargo feature in a
binary that was compiled weeks ago on a release runner. So the extra
resolves to **a second wheel**, built from a second crate with the store
clients in it.

| | base wheel | remote wheel |
|---|---|---|
| Distribution | `dynamic-config-py` | `dynamic-config-py-remote` |
| Import | `dynamic_config` | `dynamic_config.remote` |
| Size (Linux x86-64, release) | 1.36 MB | 11.68 MB |
| Extension module | `dynamic_config._core` | `dynamic_config_remote._core` |
| tokio | never | one runtime, lazily |
| MSRV | 1.85 | 1.88 — `aws-sdk-sts`, `async-nats` and `redis` each ask for it. `gix` asks for 1.85 and so moves nothing |

One wheel rather than one per store, and the size table is why. Eight
build matrices is eight times the release runner for a saving nobody has
asked for, and 11.68 MB is a wheel: `numpy` is 18 MB and `cryptography`
is 4 MB. The measurement that decided it: **etcd and Vault alone were
2.39 MB**, so the other five — the AWS SDK, `async-nats`, `redis` and two
HTTP clients — cost about 6 MB between them, and no single one of them
dominates enough to be worth splitting out on its own.

**git added 2.99 MB of that**, measured the same way: 8.69 MB before and
11.68 MB after. It is `gix` and the `reqwest` client the store crate builds
for a private authority, plus the engine's three format features — which
this wheel did not need until git, because git is the one store here that
folds several files into one document and a fold is a parse and a
re-render. It is the second largest single contributor after the AWS SDK
and still nowhere near being worth its own build matrix.

The number to revisit at is around **50 MB**, where a wheel starts to be
something people notice in a container build. If it is ever crossed, the
split to make is **S3 alone**: the AWS SDK is the largest single
contributor and the only one that brings its own credential machinery, so
`dynamic-config-py[s3]` would be a clean seam and the other seven would
stay one wheel.

## The import name

`dynamic_config.remote` is a module in the **base** wheel that re-exports
`dynamic_config_remote`, which is what the second wheel installs. Both
names work; the dotted one is the one to write.

The obvious alternative was a namespace package — the base wheel shipping
`dynamic_config/*` and the remote wheel `dynamic_config/remote/*`, one
import tree. It was built and measured before being rejected, and it
fails in two ways that matter:

- **An editable base install cannot see it.** `maturin develop` installs
  a `.pth` pointing at the source tree, so `dynamic_config.__path__` is
  that one directory and a `dynamic_config/remote/` in site-packages is
  invisible. That is how this package is developed and tested, so the
  remote wheel could never have been tested against a developed base.
- **Uninstalling the base leaves an orphan.** With `dynamic_config/__init__.py`
  gone and `dynamic_config/remote/` still there, `import dynamic_config`
  still succeeds — as a PEP 420 namespace package with no API at all.

Two distributions that never share a directory have neither problem, at
the cost of one re-exporting module. Without the second wheel, importing
that module raises an `ImportError` naming the extra rather than a
`ModuleNotFoundError` naming a distribution nobody has heard of.

## Credentials may be callables

**Every credential argument accepts a `str` or a callable returning
one.** This is the feature rather than a convenience: a configuration
watcher outlives its credentials. A Vault token has a TTL, an AppRole
secret id is rewritten by a sidecar, a workload-identity JWT is refreshed
by a daemon on the node, an etcd password is rotated by whoever rotates
passwords. A store holding the string it was constructed with is a 403
three hours later that nobody can fix without a restart.

```python
from pathlib import Path

from dynamic_config.remote import (
    S3, Auth, Consul, ConsulAuth, Etcd, Firestore, FirestoreAuth, Git,
    GitAuth, Nats, NatsAuth, Redis, Vault,
)

Etcd(
    ["http://etcd.internal:2379"], "myapp/db.json",
    user="myapp",
    password=lambda: os.environ["ETCD_PASSWORD"],
)

Vault(
    "https://vault.internal:8200", "secret", "myapp/db",
    auth=Auth.token(lambda: Path("/var/run/vault/token").read_text()),
)

Vault(
    "https://vault.internal:8200", "secret", "myapp/db",
    auth=Auth.app_role(role_id, secret_id=lambda: read_the_secret_id()),
)

Consul(
    "http://consul.internal:8500", "myapp/db.json",
    auth=ConsulAuth.token(lambda: os.environ["CONSUL_HTTP_TOKEN"]),
)

Redis(
    "redis://redis.internal:6379", "myapp/db.json",
    password=lambda: Path("/var/run/redis/password").read_text(),
)

Nats(
    ["nats://nats.internal:4222"], "config", "db.json",
    # Reads the file on every fetch, which is what an operator replacing
    # a `.creds` under a running process needs.
    auth=NatsAuth.credentials_file("/etc/myapp/nats.creds"),
)

Firestore(
    "my-project", "config/db",
    auth=FirestoreAuth.access_token(lambda: mint_a_token()),
)

S3(
    "myapp-config", "prod/db.json",
    access_key_id=lambda: current_key_id(),
    secret_access_key=lambda: current_secret(),
)

Git(
    "https://github.com/acme/config.git", "services/api/db.yaml",
    # A GitHub App installation token lives one hour, and this is the
    # store where that is the ordinary case rather than the exotic one.
    auth=GitAuth.token(lambda: mint_an_installation_token()),
)
```

The callable is invoked **on every fetch**, as ordinary Python, on the
thread that asked for the refresh — before the GIL is released for the
network read. It may block, and it may raise, in which case the refresh
fails the way any other fetch failure does: the previous document and the
previous model both keep serving.

When what it returns has **changed**, the store rebuilds its client with
the new credential. When it has not — overwhelmingly the common case —
nothing is rebuilt and the store's own token cache is untouched. That
second half matters as much as the first: rebuilding per fetch would turn
one Vault login into one per refresh, which is a poor thing to do to a
secrets store, and one Redis connection into one per refresh, which is a
poor thing to do to anything.

`S3` is the exception and the better end of the same bargain: its
credential is not inside the client at all, so a rotation rebuilds
**nothing**. [Why](#s3-the-credential-that-is-a-trait) is a section of
its own.

Two layers of renewal are at work and they do not fight. The Rust crate
already refreshes the *token* it was issued, within a minute of expiry
and reactively after a 403. What a callable adds is the layer above — the
**login credential itself** rotating, which the Rust crate cannot notice
because it was handed a `String`.

Which layer a callable belongs to differs per store, and the ones where
it is *not* needed are worth naming:

| Store | Where a callable earns its place |
|---|---|
| `Vault` | The login credential — a secret id, a password, a JWT. The Vault token is the crate's to renew |
| `Consul` | The **bearer** presented to an auth method. The ACL token is the crate's |
| `Firestore` | `FirestoreAuth.access_token`, which cannot renew. `metadata_server` renews itself and needs none |
| `Etcd`, `Redis`, `Nats` | The credential itself: it lives in the connection, so a new value is a reconnection |
| `S3` | The key pair — but nothing is rebuilt, because the SDK asks per request |
| `Git` | The https token — and nothing is rebuilt either, for a different reason: rebuilding would throw away the object database |

Three of them read a *file* instead, and deliberately:
`ConsulAuth.kubernetes(...)` and `Auth.kubernetes(...)` carry a **path**,
which the Rust crate re-reads at every login, so a projected
service-account token the kubelet rotates already works with no callable
at all — the path is `SERVICE_ACCOUNT_TOKEN`, exported so a deployment
that mounts it somewhere else can say where without spelling
`/var/run/secrets/...` from memory; `GitAuth.ssh_key(...)` is the same, one layer further out, because
`ssh` opens the key file itself at every fetch.
`NatsAuth.credentials_file(...)` is the idea one layer up: it *is* a
callable, reading the file on every fetch.

**`Git` is the second store whose rotation rebuilds nothing**, and the
reason is worth stating because it is not S3's. A `GitSource` owns a
working directory — a bare object database, filled by the first fetch —
so rebuilding one would discard every object it holds and re-transfer the
repository's whole tree, for a store whose headline property is that an
unchanged ref transfers nothing. A named `cache_dir` is worse: it is
claimed by the source that holds it, so a rebuild would be refused by the
source it was replacing. The credential is therefore a **slot** the fetch
path writes and the source reads, exactly as it is for S3, and a rotation
costs a mutex write.

That has one visible edge. A closure credential is *replaceable* as far
as the store crate is concerned, so a host that refuses one costs an
extra attempt: the source invalidates what it holds and tries once more,
and the slot answers with the same value because Python resolved it for
this fetch already. One wasted round trip on a refusal, in exchange for a
rotation that costs no transfer.

## Credentials never appear in a diagnostic

Not in an exception message, not in a `repr`, not in `describe()` — which
is what `Origin` records and what every remote error carries.

```python
>>> Etcd(["http://app:hunter2@etcd.internal:2379"], "myapp/db.json")
<etcd http://app:***@etcd.internal:2379 key myapp/db.json>

>>> Auth.app_role("role-id", "hunter2-secret-id")
Auth.app_role('role-id', '***')
```

Two rules, and they are belt and braces:

- **A URL loses its password**, by the rule the Rust store crates use
  rather than a second copy of it: split on the *last* `@`, because a
  password may itself contain one. The user name survives, because
  redaction that hides the half worth seeing is redaction nobody can
  debug through.

  The one thing the stores disagree about is what an authority with **no
  colon** in it means, and the shared rule takes it as an argument rather
  than being forked: `nats://token@host` is a *secret*, and
  `redis://user@host` is a *user name*.

  ```python
  >>> Nats(["nats://hunter2@nats.internal:4222"], "config", "db.json")
  <nats nats://***@nats.internal:4222 bucket config key db.json>
  >>> Redis("redis://app@redis.internal:6379", "myapp/db.json")
  <redis redis://app:***@redis.internal:6379 key myapp/db.json>
  ```
- **Every credential resolved for a call is scrubbed by value** from
  anything that call reports. The URL rule only reaches a credential that
  is *in* a URL; this one reaches a client library that decided to be
  helpful in a version nobody here has read.

The non-secret half of an auth method stays printable, and the split is
the one the Rust crate's `Debug` makes: an AppRole role id and a userpass
user name are shown, the secret id and the password are not.

## TLS: a private authority and a client certificate

A deployment behind an **internal certificate authority** — an enterprise
CA, a TLS-inspecting proxy, a MinIO with its own certificate, a GitLab on
the company's own root — needs to trust one more certificate than the
platform does. A hardened one needs to *present* one as well. Every store
here takes both, spelled the same way, through one type:

```python
from dynamic_config.remote import Auth, TlsConfig, Vault

vault = Vault(
    "https://vault.internal:8200", "secret", "myapp/db",
    auth=Auth.kubernetes("myapp"),
    tls=TlsConfig()
        .with_ca_certificate_file("/etc/ssl/private-ca.pem")
        .with_client_certificate_files("/etc/ssl/app.crt", "/etc/ssl/app.key"),
)
```

This is the one client-configuration surface that crosses into Python at
all, and that is a property of how it was built rather than of how hard
anyone tried. `TlsConfig` is `dynamic_config_store_core::tls::TlsConfig`,
which holds **paths and PEM bytes and nothing else** — no `tonic`
configuration, no `ureq::Agent`, no `SdkConfig` anywhere in it. A surface
made of data has a Python spelling; a surface made of a client's own types
does not, which is why [everything else on that list](#what-is-not-exposed)
is still on it.

Four settings, in two spellings each:

| | |
|---|---|
| `TlsConfig()` | The platform's own trust store, no client certificate |
| `.with_ca_certificate_file(path)` | Trust the authority in this PEM file |
| `.with_ca_certificate_pem(bytes)` | Trust the authority in these PEM bytes |
| `.with_client_certificate_files(certificate, key)` | Present this certificate and key (mTLS) |
| `.with_client_certificate_pem(certificate, key)` | The same, from bytes |
| `.is_empty()` | Whether this asks for anything at all |

Both spellings exist because both deployments do. A file is what a
Kubernetes secret mount or an `/etc/ssl` layout produces; bytes are what a
program that already fetched its material from a secrets manager has, and
writing those to a temporary file so a client could read them back would
put a private key on a disk that never asked for one.

Three smaller things hold everywhere. An **empty** `TlsConfig` is not "no
TLS" — it is the platform's trust store, so `tls=None` and
`tls=TlsConfig()` are the same store. A **named authority replaces** the
platform trust store rather than joining it, because that is what pinning
means; a deployment needing both puts both in the one file — with one
exception, `Git`, where the authority is **added** to the platform's, so
that one source configuration reaches both a private GitLab and
github.com. And **nothing is read at construction**: the files are opened when the store builds its
client, so a missing certificate is an error naming the path at the first
refresh rather than an exception in the middle of building a
configuration.

`Pem`, `ClientCertificate` and `CertificateAndKey` are **not** bound. They
exist in Rust so a store can ask *which spelling was this*; a Python caller
never asks — they say what they have — so binding them would add three
names to the surface and three more places a private key could be
rendered.

### What each store accepts, and what two of them refuse

| Store | CA from a file | CA from bytes | Client certificate |
|---|---|---|---|
| `Consul`, `Etcd`, `Firestore`, `Redis`, `Vault` | yes | yes | yes |
| `Git` | yes | yes | yes — on an `https://` url, and **refused** on any other |
| `Nats` | yes | **no** — `async-nats` opens the file itself | file paths only |
| `S3` | yes | yes | **no** — the SDK's TLS context has no slot for one |

**The refusals are refusals, not omissions.** A binding that quietly
ignored either would leave a program believing it had pinned a private
authority when it had not, which is worse than a program that will not
start — so both raise a `ValueError` at construction, in the store crates'
own wording, naming the call and the way out:

```python
>>> Nats(["tls://nats.internal:4222"], "config", "db.json",
...      tls=TlsConfig().with_ca_certificate_pem(ca_bytes))
ValueError: nats tls://nats.internal:4222 bucket config key db.json: a
certificate authority from PEM bytes cannot be expressed here, and is
refused rather than ignored; `async-nats` opens the file itself; name a
file with `with_ca_certificate_file`

>>> S3("myapp-config", "prod/db.json",
...    tls=TlsConfig().with_client_certificate_files("app.crt", "app.key"))
ValueError: s3 myapp-config/prod/db.json: a client certificate cannot be
expressed here, and is refused rather than ignored; the AWS SDK's TLS
context has a trust store and no client-certificate slot; mTLS to an
S3-compatible server means building the connector, which only the Rust
crate can do
```

`Git`'s is the same rule pointed at a different mistake. It expresses all
four settings, and only over `https://`: an `ssh://` remote authenticates
its host through `known_hosts` and its client through a key, so a
certificate authority has nothing to do with it and is **refused rather
than half-applied**.

```python
>>> Git("ssh://git@github.com/acme/config.git", "db.yaml",
...     auth=GitAuth.ssh_agent(),
...     tls=TlsConfig().with_ca_certificate_file("/etc/ssl/private-ca.pem"))
ValueError: git: `tls` configures the https transport and this url is not
an https one, so it cannot be applied here and is refused rather than
ignored; an ssh remote authenticates its host through `known_hosts` and
its client through a key — GitAuth.ssh_agent(), GitAuth.ssh_key(path) or
GitAuth.ssh_command(command)
```

Its refusals are the one place this wheel writes its own wording rather
than borrowing the store crate's, and for a reason: **a git remote url
routinely carries a token**, the redaction that removes one lives in Rust,
and a message assembled in Python has no way to reach it. So these
messages name the argument and never the url. The store crate refuses the
same configuration again when it builds, which is the belt to this
braces.

At construction rather than at the first fetch, and before the tokio
runtime is started: a configuration a store cannot express should be an
error at the line it was written on, not two worker threads and a failure
one refresh later. The Rust crates refuse the same thing again when they
build their client, which is the belt to this braces.

Two stores have one more thing worth knowing. **`Nats` turns TLS on** when
an authority is named — the store crate sets `require_tls`, so a `nats://`
URL that would have negotiated plaintext fails rather than quietly
connecting without the authority just named. And **`Redis` needs a
`rediss://` URL**: TLS material on a `redis://` one is refused too, but by
the client as it is built, so that arrives as a `RemoteError` at the first
fetch. The URL is parsed where it is used rather than twice by two
implementations that could disagree.

One consequence of how the wheel is compiled: it enables the `tls` feature
of `dynamic-config-etcd` and `dynamic-config-redis`, which is what makes
`Etcd` and `Redis` able to speak TLS at all — an extra cannot turn on a
Cargo feature, so the choice is made once, here, and costs about 0.2 MB in
the wheel every install carries. It does **not** enable
etcd's `tls-roots`, which would resolve to a `tonic` call the store crate
never makes and add a native-certificate crate to every wheel for no
change in behaviour. So etcd over TLS from Python trusts the authority you
name, and a client certificate for etcd goes with one.

### The private key never appears

It is the sharpest secret this package handles, and `repr` is where a
`repr` usually leaks one. `TlsConfig.__repr__` **delegates to the Rust
type's own `Debug`** rather than rendering the arguments again:

```python
>>> TlsConfig().with_client_certificate_files("/etc/ssl/app.crt", "/etc/ssl/app.key")
TlsConfig { ca_certificate: None, client_certificate: ClientCertificate {
certificate: file /etc/ssl/app.crt, key: "file /etc/ssl/app.key" } }

>>> TlsConfig().with_client_certificate_pem(certificate, private_key)
TlsConfig { ca_certificate: None, client_certificate: ClientCertificate {
certificate: <pem bytes>, key: "<redacted>" } }
```

A **path** is printed, because it names which key and is the question
somebody debugging this is actually asking; **bytes** are withheld, because
they are the key. One implementation of that rule, in the crate that owns
the type, with a planted-key test over it — a second copy here would be the
one nobody thinks to check. The remote wheel's suite plants a private key
of its own and greps every diagnostic this surface can produce: each
store's `repr` and `describe()`, what the engine records as provenance, and
the text of both refusals.

There is **no way to turn verification off**, and the
[Rust chapter](../remote-stores.md#there-is-no-way-to-turn-verification-off)
argues that at length. The short version: it could not be uniform, it
answers nothing `with_ca_certificate_file` does not, and every client
underneath still has its own dangerous switch under its own frightening
name for the case nobody anticipated.

## The tokio runtime

One, lazily, owned by the module.

Three of the eight are async — etcd speaks gRPC, NATS has its own
protocol, and the AWS SDK is async throughout — so something has to drive
their futures. The other five are `ureq`, a plain socket or a git fetch:
blocking, no executor anywhere in them, and no runtime at all.

```python
>>> from dynamic_config.remote import Auth, Etcd, Vault, runtime_started
>>> runtime_started()                                    # import starts nothing
False
>>> Vault("https://vault:8200", "secret", "a/b", auth=Auth.token("t"))
>>> runtime_started()                                    # nor does a blocking store
False
>>> Etcd(["http://etcd:2379"], "myapp/db.json")
>>> runtime_started()                                    # this one needs it
True
```

It starts at **construction** rather than at the first fetch, because
construction is the moment a user can observe; a reactor appearing on a
later network call is a thread count nobody can explain. It has two
worker threads — the work is one request per configuration refresh, and
sizing it to the machine would put sixty-four parked threads in a
container that reads one key.

It is **multi-threaded rather than current-thread** so that two Python
threads refreshing two stores do not serialise behind each other, and it
is **never shut down**. `Runtime::drop` blocks until its workers park;
registering that at `atexit` would put a join between the interpreter and
its own exit, behind whatever a worker happens to be doing — which for a
store is a network read with a ten-second deadline on it. The base wheel
already registers an `atexit` sweep, and two that can each block would be
ordered by registration accident.

An immortal runtime is dangerous in a binding whose tasks call Python, and
it is safe here because **no task on this runtime ever touches Python**:
credentials are resolved in Python before the call, and the futures driven
here hold nothing but Rust. A worker still running while CPython finalises
cannot re-enter a dying interpreter.

That invariant is what shapes S3's credential provider, which is the one
piece of this wheel the SDK calls back into: it reads a value the fetch
path already resolved, and never calls Python.

If the calling thread is *already* inside a tokio runtime — a Rust program
embedding CPython, calling in from a task — `block_on` would panic with
*Cannot start a runtime from within a runtime*. That case is detected with
`Handle::try_current()` and the future is handed to this wheel's own
workers instead: a different runtime, so no reentrancy and no deadlock.

`set_executor` is unchanged and still means what it meant: which Python
pool pays for the blocking half. `refresh_remote_async()` runs the whole
fetch on that pool, and the tokio runtime is what the fetch uses once it
gets there.

## S3: the credential that is a trait

Seven of the eight stores take their credential as a string, so a callable
resolves to a string and the string is handed over. The AWS SDK does not.
Its credential surface is a **`ProvideCredentials` implementation**, asked
per request, from inside the SDK's own async machinery — and that shape is
the reason S3 was the first store here with a design to make rather than a
builder to mirror. `Git` was the second, and it reuses this one: the slot
below is the shape its credential takes too, for a different reason
([above](#credentials-may-be-callables)).

**Passing no credentials is a first-class mode**, not a fallback. With
`access_key_id` and `secret_access_key` absent, the SDK's own chain runs
untouched: environment variables, the shared profile, the EC2 instance
role, the ECS task role, IRSA on EKS. On anything running inside AWS that
is the right answer, and a second credential chain in a program that
already has one is a bug waiting for a rotation.

**When they are passed, the callable becomes a shim.** A provider is
installed in the chain's place which answers from a slot the fetch path
writes:

```text
S3.fetch()
  → resolves access_key_id / secret_access_key   Python, on the caller's thread
  → writes them into the slot                    still holding the GIL
  → py.detach                                    GIL released
      → the SDK signs a request
          → asks the provider                    a mutex read; no Python
```

The alternative — a provider that called the Python callable when the SDK
asked — was rejected for a reason the [runtime](#the-tokio-runtime)
section already states: **nothing on that runtime may touch Python.** The
runtime is never shut down, which is safe precisely because a worker
outliving the interpreter holds nothing but Rust. A provider acquiring the
GIL from a tokio worker would have turned the one safe immortal runtime
here into the dangerous kind.

Two consequences fall out of the slot, and both are improvements:

- **A rotated key rebuilds nothing.** Six of the other seven rebuild a
  client when their credential moves, because the credential is inside
  it. S3's is not: the next request signs with the new value, and the
  connection pool and the resolved endpoint are untouched. (`Git` is the
  seventh, and does not rebuild either — it has an object database to
  protect rather than a connection pool.)
- **The SDK's identity cache has to be off.** It exists to keep a provider
  that calls IMDS from being called per request, and it caches a
  credential with no expiry *indefinitely* — which for a deliberately
  mutable slot would mean the first key signing every request forever.
  `IdentityCache::no_cache()` costs nothing here, because the provider it
  defeats is a mutex read.

## What is not exposed

- **The clients' own configuration types.** `etcd`'s `ConnectOptions`,
  Vault's, Consul's and Firestore's `ureq::Agent`, NATS' `ConnectOptions`
  and S3's `SdkConfig` are all taken by their Rust builders deliberately,
  so that options this project has never heard of keep working. There is
  no Python spelling for a `tonic` configuration or a `ureq` agent, so a
  **custom proxy**, a hand-built connector or a DNS resolver belongs to a
  deployment that uses the Rust crate directly.

  [TLS](#tls-a-private-authority-and-a-client-certificate) used to be on
  this list and no longer is, and the difference is the shape of the
  surface rather than the effort: a certificate authority and a client
  certificate are *data*, so they cross. Anything still here is a client
  type, and a client type does not.

  One of those types also holds a credential kind that is therefore
  absent: NATS' JWT-with-signing-callback.
- **Several keys as one document — for the seven that are not git.**
  Consul, Redis and the rest can read a list of keys or a whole prefix and
  merge them; here each of them reads **one key**. That is a second
  vocabulary — a merge order, an overlap rule — and for a key/value store
  it is also a document that never existed at any instant, because the set
  is one request per key. A Python deployment that needs it can merge two
  sources instead, which is what the layering is already for.

  **`Git` is the exception, and the reason is the object model rather
  than the effort.** One fetch resolves one commit, and a commit has one
  tree, so a list of paths or a whole directory is read as of one instant
  with nothing arranged for it: no transaction, no listing race, no second
  round trip. So `GitKeys` is bound and the other seven still read one
  key.
- **`watch()`.** Every crate here can watch its store and push changes.
  That is a Rust callback on a Rust thread calling into Python, which is a
  second GIL story on top of this one; `refresh_remote()` on a timer is
  what Python has, and it is what the base wheel's remote path already
  offers.

  **Git is where that costs something**, and it is left costing it on
  purpose. It is the only store in the family whose *multi-file* sources
  can be watched at all — what moves is a ref, and what a ref names is a
  commit, so a watch wakes on the repository rather than on one file and
  the re-read that follows takes every file out of that one commit. Making
  git the exception would mean the second GIL story for one store, and a
  binding whose watch works for one of eight is a worse surface than one
  whose watch is absent for all eight. Against git a poll is cheap
  anyway: each tick is one ref advertisement, and only a ref that moved
  costs a transfer.
- **A credential with a lifetime the issuer stamped on it.**
  `Credential::expiring` hands the Rust crate a token *and its TTL*, so it
  is refreshed within a minute of expiry and after a refusal rather than
  per fetch. Python has the per-fetch shape only — every credential
  argument is called on every fetch — because the caching would have to
  live in Python anyway: nothing on a fetch may call back into the
  interpreter, so a Rust-side cache could not invoke a Python closure. A
  caller who mints an installation token per hour caches it in their own
  closure, which is four lines and is where the exchange already lives.
- **`from_client` / a shared connection.** Nothing in a Python process is
  holding an `etcd_client::Client` or an `aws_sdk_s3::Client` to share.
- **A Firestore service-account JSON key.** Absent in the Rust crate too,
  and as a recommendation rather than a gap: signing one means an RS256
  stack in a configuration library, and Google's own guidance is that a
  downloaded key is the option of last resort.

## API

### `Etcd(endpoints, key, *, format=None, timeout=10.0, user=None, password=None, tls=None)`

A key in an etcd v3 store. Mirrors `dynamic_config_etcd::Etcd`.

| Argument | Meaning |
|---|---|
| `endpoints` | etcd's, as `http://host:port`. At least one |
| `key` | The key whose value is the configuration document |
| `format` | `"json"`, `"toml"` or `"yaml"`. Read from the key's extension when it has one |
| `timeout` | The deadline for **one fetch attempt**, in seconds |
| `user`, `password` | etcd's own authentication. Either both or neither; each may be a callable |
| `tls` | A [`TlsConfig`](#tlsconfig): a private certificate authority, a client certificate, or both |

Nothing connects at construction — etcd's client connects lazily, and a
bad endpoint surfaces at the first refresh, exactly as in Rust.

### `Vault(address, mount, path, *, auth, key="db", namespace=None, timeout=10.0, tls=None)`

A secret in Vault's KV v2 store. Mirrors `dynamic_config_vault::Vault`.

| Argument | Meaning |
|---|---|
| `address` | `https://vault.internal:8200` |
| `mount`, `path` | The secret at `{mount}/{path}` |
| `auth` | An [`Auth`](#auth). Required — a `Vault` with no credentials could only ever produce a 403 |
| `key` | The section key the secret is wrapped under. Must match the configuration's |
| `namespace` | The Vault Enterprise namespace, if there is one |
| `timeout` | The deadline for **one fetch attempt**, in seconds |
| `tls` | A [`TlsConfig`](#tlsconfig): a private certificate authority, a client certificate, or both |

Vault stores a section's *contents* rather than a whole document, which
is why `key` exists and why it has to agree with the one
`DynamicConfig(..., key=...)` was given.

### `Auth`

How to obtain a Vault token — one classmethod per Rust variant. Every
credential argument accepts a callable. It is `Auth` rather than
`VaultAuth` because Vault's shipped first and an installed wheel already
imports it under that name; the three that followed are named for their
store. Redis and S3 have none, because neither has a login.

| | |
|---|---|
| `Auth.token(token)` | A token somebody already obtained |
| `Auth.app_role(role_id, secret_id)` | AppRole, on the `approle` mount |
| `Auth.kubernetes(role)` | The pod's service-account token, on `kubernetes` |
| `Auth.jwt(jwt)` | JWT/OIDC, on `jwt` |
| `Auth.userpass(username, password)` | On `userpass` |
| `Auth.ldap(username, password)` | On `ldap` |
| `Auth.certificate()` | A TLS client certificate, on `cert` |

| Modifier | Effect |
|---|---|
| `at_mount(path)` | A different mount path. No effect on `token`, which has none |
| `with_role(role)` | For `kubernetes`, `jwt` and `certificate` |
| `with_token_path(path)` | For `kubernetes` |

Each returns a **new** `Auth` rather than mutating: one shared between
two stores cannot be changed by either.

### `Consul(address, key, *, format=None, auth=None, datacenter=None, timeout=10.0, tls=None)`

A key in Consul's KV store. Mirrors `dynamic_config_consul::Consul`.

| Argument | Meaning |
|---|---|
| `address` | The agent's, as `http://host:8500` |
| `key` | The key whose value is the whole configuration document |
| `format` | Read from the key's extension when it has one |
| `auth` | A [`ConsulAuth`](#consulauth). Defaults to `anonymous()`, which is what a Consul with ACLs disabled wants |
| `datacenter` | One that is not the agent's own |
| `timeout` | The deadline for **one fetch attempt**, in seconds |
| `tls` | A [`TlsConfig`](#tlsconfig): a private certificate authority, a client certificate, or both |

Consul stores an opaque blob, so the value is a whole document — the
opposite of `Vault`, which wraps a secret's fields under a section key.

### `ConsulAuth`

| | |
|---|---|
| `ConsulAuth.anonymous()` | No token. ACLs disabled, or a readable `default` policy |
| `ConsulAuth.token(token)` | Usually `CONSUL_HTTP_TOKEN` |
| `ConsulAuth.kubernetes(method)` | The pod's service-account token, presented to an auth method |
| `ConsulAuth.jwt(method, token)` | A JWT or OIDC id token |
| `.with_bearer_file(path)` | Reads the bearer from a file, re-read at every login |
| `.with_meta(name, value)` | Consul's `Meta`, for the audit log |

### `Firestore(project, path, *, auth, key="db", database="(default)", endpoint=None, timeout=10.0, tls=None)`

A document in Firestore. Mirrors `dynamic_config_firestore::Firestore`.

| Argument | Meaning |
|---|---|
| `project`, `path` | The GCP project, and collection-then-document — `config/db` |
| `auth` | A [`FirestoreAuth`](#firestoreauth). Required |
| `key` | The section key the document is wrapped under. Must match the configuration's |
| `database` | One that is not `(default)` |
| `endpoint` | What the emulator needs — `http://127.0.0.1:8080` |
| `timeout` | The deadline for **one fetch attempt**, covering the token fetch too |
| `tls` | A [`TlsConfig`](#tlsconfig): a private certificate authority, a client certificate, or both |

`auth` is required where the Rust builder defaults to the emulator: *send
no credentials* is a reasonable default for a builder being filled in and
a poor one for a constructor, where it would quietly produce a 401 against
the real service.

### `FirestoreAuth`

| | |
|---|---|
| `FirestoreAuth.metadata_server()` | The workload's own identity: GKE, Cloud Run, GCE. Renews itself |
| `FirestoreAuth.access_token(token)` | Anything that already has one. Cannot renew — this is the one a callable is for |
| `FirestoreAuth.emulator()` | No token at all |
| `.with_url(url)` | A sidecar's metadata address |

### `Git(url, path, *, branch=None, tag=None, commit=None, format=None, auth=None, cache_dir=None, timeout=None, max_bytes=None, compact_after=None, tls=None)`

A file — or a set of them — in a git repository. Mirrors
`dynamic_config_git::GitSource`.

| Argument | Meaning |
|---|---|
| `url` | Anything git understands: `https://…`, `ssh://…`, `git@host:org/repo.git`, or a local path |
| `path` | One `/`-separated path relative to the repository root, a list of them, or a [`GitKeys`](#gitkeys) |
| `branch`, `tag`, `commit` | Three spellings of one reference — **name at most one**. `main` when none is named; a commit is the full hexadecimal object id |
| `format` | Read from a path's extension when it has one. Required for a name that does not say, for a list naming two formats, and always for a directory |
| `auth` | A [`GitAuth`](#gitauth). Defaults to `anonymous()`, which is what a public repository wants |
| `cache_dir` | Keeps the object database somewhere that survives restarts. Two sources may not name one directory |
| `timeout` | The deadline for **one fetch attempt**. The Rust crate's thirty seconds when absent |
| `max_bytes` | The largest **single file** that will be read. A megabyte when absent |
| `compact_after` | Transfers a working directory may accumulate before it is emptied and refilled. Thirty-two when absent; `0` never empties it |
| `tls` | A [`TlsConfig`](#tlsconfig) — on an `https://` url, and **refused** on any other |

A fetch is **shallow and single-ref**: the ref advertisement, then that one
commit at depth 1 if it is not already held, then one blob read out of its
tree. Nothing is ever checked out, so a repository containing a symlink to
`/etc/shadow` cannot make a checkout that never happens write anywhere. **An
unchanged ref transfers nothing**, which is what makes polling a git host
reasonable — and what the first fetch costs is the repository's whole tree,
because a commit's tree is what the protocol delivers.

`describe()` names the **commit** once one has been read, and the ref that was
asked for until then: *which commit is this program actually serving* is the
first question of every configuration-in-git incident, and a branch name does
not answer it.

Three things are refused at construction rather than half-applied, each naming
the call and the way out: **two references** (three keyword arguments have no
order, where Rust's three builder calls do), **a credential for the transport
this url does not use** (an ssh key on an `https://` remote is not
half-configured, it is silently anonymous), and **`tls` on a url with no TLS in
it**.

The default working directory is a **private temporary one**, `0700` from the
moment it exists, removed with the store. It is the one construction in this
wheel that touches the filesystem, and it touches no network.

### `GitKeys`

What a source reads. Mirrors `dynamic_config_git::Keys`; a bare string is
`one` and a list of strings is `several`, so only a directory needs the import.

| | |
|---|---|
| `Git(url, "services/api/db.yaml")` | One file, handed to the loader byte for byte |
| `Git(url, ["base.yaml", "local.yaml"])` | Several, merged **in the order given — later wins** |
| `GitKeys.prefix("services/api")` | Every file under a directory, merged as **disjoint sections** — an overlap is a deployment bug and is reported as one |

A directory, not a string prefix: `prefix("services/api")` reads
`services/api/db.yaml` and does not read `services/api-old.yaml`. It needs
`format`, because a directory has no extension to read one from.

It is `GitKeys` rather than `Keys` because eight stores share one namespace and
only this one has it — see [what is not exposed](#what-is-not-exposed) for why
the other seven read a single key.

### `GitAuth`

How to authenticate to a git host — one classmethod per Rust `Credential`
constructor. git has exactly two places a credential can go, so this is those
two plus the absence of both.

| | |
|---|---|
| `GitAuth.anonymous()` | A public repository |
| `GitAuth.token(token)` | A PAT, an installation token, a deploy token. Travels as basic auth with `x-access-token` in the user half |
| `GitAuth.basic(username, password)` | For the host that reads the user half — a GitLab deploy token, `gitlab-ci-token` with `CI_JOB_TOKEN` |
| `GitAuth.ssh_agent()` | Whatever `ssh` would do unaided: `SSH_AUTH_SOCK`, `~/.ssh/config`, a `ProxyJump`, a hardware key |
| `GitAuth.ssh_key(path)` | One private key file, with `-o IdentitiesOnly=yes` so an agent cannot offer others first |
| `GitAuth.ssh_command(command)` | Run this instead of `ssh`. Redacted whole, because it may be carrying a secret |

The `ssh` **binary must be on the host** for the last three: `gix` carries an
SSH stream by spawning it, exactly as `git` does — and in exchange everything
already configured for `ssh` works.

`ssh_key` takes a path and no callable, deliberately: `ssh` opens the file at
every fetch, so a key an operator replaces is picked up already. **A passphrase
is not accepted in any spelling** — `ssh` has no way to take one that does not
put it on a command line where `ps` can read it, so a passphrase-protected key
belongs in an agent.

### `Nats(servers, bucket, key, *, format=None, auth=None, timeout=10.0, tls=None)`

A key in a JetStream KV bucket. Mirrors `dynamic_config_nats::Nats`.

| Argument | Meaning |
|---|---|
| `servers` | NATS URLs, as `nats://host:4222`. A list, because a cluster is ordinary |
| `bucket`, `key` | The KV bucket, and the key in it |
| `format` | Read from the key's extension when it has one |
| `auth` | A [`NatsAuth`](#natsauth). Defaults to `anonymous()` |
| `timeout` | The deadline for **one fetch attempt**, in seconds |
| `tls` | A [`TlsConfig`](#tlsconfig) — **file paths only**, and naming an authority turns TLS on |

Nothing connects at construction, which is a **difference from the Rust
crate**: `Nats::with_options` connects and resolves the bucket in its
constructor. Here construction touches no network, like every other store
in this wheel. The bucket is never created.

### `NatsAuth`

| | |
|---|---|
| `NatsAuth.anonymous()` | No credential |
| `NatsAuth.token(token)` | What `nats://token@host` carries, as an argument |
| `NatsAuth.user_and_password(user, password)` | |
| `NatsAuth.nkey_seed(seed)` | The `SU…` half, which signs the server's nonce |
| `NatsAuth.credentials(contents)` | A `.creds` file's **contents** |
| `NatsAuth.credentials_file(path)` | The same, read on every fetch |

### `Redis(url, key, *, format=None, user=None, password=None, timeout=10.0, tls=None)`

A key in Redis. Mirrors `dynamic_config_redis::Redis`.

| Argument | Meaning |
|---|---|
| `url` | `redis://host:6379`, or `rediss://` for TLS |
| `key` | The key whose value is the whole configuration document |
| `format` | Read from the key's extension when it has one |
| `user`, `password` | Each may be a callable. `password` alone is `requirepass`, which implies the default user |
| `timeout` | The deadline for **one fetch attempt** — connecting, writing and waiting |
| `tls` | A [`TlsConfig`](#tlsconfig). Needs a `rediss://` URL, and material on a `redis://` one is refused at the first fetch |

The credentials are arguments rather than part of `url` on purpose: a
callable cannot rotate a substring of a string somebody passed at
construction. They are spliced into the authority, percent-encoded, and
they replace any the URL already carried.

### `S3(bucket, key, *, format=None, region=None, endpoint=None, access_key_id=None, secret_access_key=None, session_token=None, timeout=10.0, tls=None)`

An object in S3. Mirrors `dynamic_config_s3::S3`.

| Argument | Meaning |
|---|---|
| `bucket`, `key` | The object, whose body is a whole configuration document |
| `format` | Read from the key's extension when it has one |
| `region`, `endpoint` | Resolved from the environment when absent. `endpoint` is what reaches MinIO, Ceph, R2 or B2 |
| `access_key_id`, `secret_access_key` | Both or neither; each may be a callable. Absent means [the SDK's own chain](#s3-the-credential-that-is-a-trait) |
| `session_token` | For assumed credentials. Needs the pair |
| `timeout` | The deadline for **one fetch attempt** — and the SDK retries, so a fetch can take this three times over |
| `tls` | A [`TlsConfig`](#tlsconfig) — **a certificate authority only**; a client certificate is refused |

Path-style addressing is always on, because the virtual-host form needs
DNS entries only AWS has.

### `TlsConfig`

A private certificate authority and a client certificate, as data. Mirrors
`dynamic_config_store_core::tls::TlsConfig` method for method, and every
store takes one as `tls`. Each method answers a **new** `TlsConfig`, so one
shared between two stores cannot be changed by either.

| | |
|---|---|
| `TlsConfig()` | The platform's own trust store, no client certificate |
| `.with_ca_certificate_file(path)` | Trust the authority in this PEM file. `str` or `os.PathLike` |
| `.with_ca_certificate_pem(pem)` | Trust the authority in these PEM `bytes` |
| `.with_client_certificate_files(certificate, key)` | Present this certificate and key (mTLS) |
| `.with_client_certificate_pem(certificate, key)` | The same, from `bytes` |
| `.is_empty()` | Whether this asks for anything at all |

`repr()` is the Rust type's own `Debug`: a path where there is one,
`<redacted>` where the key is bytes, and never the material.
[The section above](#tls-a-private-authority-and-a-client-certificate) has
what each store accepts, what `Nats` and `S3` refuse, and why refusing is
the only honest answer.

### `runtime_started()`

Whether the tokio runtime has been started. It exists so the promise
above is testable rather than merely stated.

### `Credential`

`Union[str, Callable[[], str]]` — the type every credential argument
takes. Not a coroutine: the fetch path is synchronous by construction, so
an `async def` here would never be awaited by anything.

## Errors

A failed fetch raises the base wheel's exceptions, not a second set:

| | |
|---|---|
| `dynamic_config.AuthError` | The credential was refused. Waiting will not fix it |
| `dynamic_config.RemoteError` | Anything else. Waiting might |

The compiled half raises its own pair — it is a different extension
module and cannot raise the base wheel's class objects — and the Python
facade translates, with the original attached as `__cause__`. Unlike a
store written in Python, the *message* is repeated here, because this
wheel wrote it and has already taken the credentials out.

## Testing without a store

The scripted servers in this project's own suite are the pattern: a
`ThreadingHTTPServer` speaking enough of Vault's KV v2 API — or Consul's,
or Firestore's, or S3's `GET` — to be read by the real client, whose
request log is then asserted on. It is the only way to prove a credential
*rotation*: the claim is about which bytes reached the server, so only a
server that records them can settle it.

Four of the eight can be scripted that way because they speak HTTP. Three
speak binary protocols — etcd's gRPC, NATS', Redis' RESP — where a
scripted server would be a protocol implementation rather than a fixture,
so those prove the same claim against a **real** server, by rotating from
a credential it refuses to one it accepts: the first fetch raises
`AuthError`, the second returns the document, and the same store object
did both.

**Git is scripted and real at once.** Its host is a `ThreadingHTTPServer`
that checks the `Authorization` header and hands the git half to a real
`git upload-pack --stateless-rpc` over a real repository — so the protocol
is the one GitHub serves, and the token is the one a test chose. It is
served over TLS behind a throwaway authority rather than over plain HTTP,
and not for symmetry: `gix` refuses to put a credential on an unencrypted
connection unless it is compiled with
`gix-transport/http-client-insecure-credentials`, which this wheel is not
and should not be, so a scripted host over `http://` would never be shown
a token at all. Everything that is not authentication is read over
`file://` from a repository the suite builds with `git` — no network, no
container, no mock of the thing under test.

S3's proof is worth naming separately, because S3 has no login and no
reconnection to count. What names the credential there is the SigV4
`Authorization` header — `Credential=AKID/…` — so the scripted server
reads the access key id out of it, and the rotation is the two ids it
saw. The containers prove the protocol; the scripted servers prove the
rotation.
