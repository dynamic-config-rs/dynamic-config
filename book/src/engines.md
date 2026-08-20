# Engines and Readers

Two steps of a load are a choice rather than a fact: which parser turns a
document into values, and which fold turns the layers into one
configuration.

## Engines

The **engine** is the step that folds one tree per layer into one
configuration. Everything before it — discovery, decryption, sections, the
environment, `.env` files, `--set` — is this crate's and no engine sees a
file. Everything after it — aliases, validation, the snapshot, the reload —
is this crate's too.

Two ship, and a third is anything implementing the trait:

| engine | feature | what it is |
|---|---|---|
| `config_rs()` | — | the fold of the [`config`](https://docs.rs/config) crate; **the default** |
| `figment()` | `figment` | the fold of the [`figment`](https://docs.rs/figment) crate |

**This crate has no fold of its own.** It had one — the reference the
others were compared against — and carrying a second implementation of a
rule somebody else already implements is maintenance with no reader. The
rule is still written down here and still held by the tests; what is gone
is a third copy of it.

### Switching the engine

Per load, which is the narrow way:

```rust
let config: Db = DbConfig::builder("db")
    .file("config.toml")
    .engine(dynamic_config::engine::figment())
    .load()?;
```

Or once for the process, before the first `init()`:

```rust
dynamic_config::engine::set_engine(dynamic_config::engine::figment())?;
```

A load that names its own engine uses that one whatever is installed. With
nothing named and nothing installed, `config-rs` runs.

### What switching an engine does not change

**The merge rule is the same in all of them:** tables descend, everything
else replaces, arrays included. So is the precedence order, which is decided
before an engine is called, and so is provenance — `explain()` and
`source_of()` name the same winner whichever engine ran.

That is a claim about behaviour, so it is tested as one. The whole
composition corpus, a generated corpus of layer stacks, and the corner cases
where a backend's habits could show through are all run through every engine
and compared on the tree **and** on the winner of every leaf.

Two of those corners are worth naming, because they are what the adapters
exist to absorb:

- **A key is a name, never a path.** `{"my.module": "debug"}` is one key with
  a dot in it, which is how a great deal of logging configuration is
  written. `config` reads a top-level key as a path expression and would
  answer `{"my": {"module": "debug"}}`; its adapter hands over stand-in
  names and puts the document's own back afterwards.
- **Provenance survives an engine that does not track it.** `figment`
  records metadata per provider and cannot answer for a key with a dot in
  it; an engine of your own may report nothing at all. The fold fills the
  gap from the layers it was given — under the merge rule, the winner of a
  leaf *is* the highest layer supplying that path, so this is read off the
  input rather than guessed at.

### Writing an engine

An engine is one method. This is a complete, working one:

```rust
use dynamic_config::engine::{Engine, Folded, Layer};
use dynamic_config::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Mine;

impl Engine for Mine {
    fn name(&self) -> &str {
        "mine"
    }

    fn fold(&self, layers: &[Layer<'_>]) -> Result<Folded, dynamic_config::Error> {
        let mut values = Value::Table(BTreeMap::new());

        for layer in layers {
            values.merge(layer.values.clone());
        }

        Ok(Folded { values, tags: BTreeMap::new() })
    }
}

static MINE: Mine = Mine;
```

Reach it the same way as the two that ship:

```rust
let config: Db = DbConfig::builder("db").file("config.toml").engine(&MINE).load()?;
```

#### The contract

Four things are asked of an implementation. The rest is its business.

1. **Precedence is the argument's order.** `layers[0]` is the lowest, and
   the last one wins. The order was decided before you were called and is
   part of this crate's API.
2. **A tag is reported only for a leaf that layer actually supplied.** The
   tag is opaque and belongs to the caller — it is an index into origins you
   cannot see — so an engine that invents one is reporting a file or a
   variable that does not exist. Reporting *no* tags is allowed and costs
   nothing a reader can see; see the next section.
3. **An error never carries a configuration value.** A backend's own message
   for a type error usually quotes what it found, and what it found is as
   likely to be a password as anything in a configuration ever is. Say the
   shape, the key, or nothing.
4. **The fold is a function of its layers.** Same layers, same answer, every
   time. A reload compares the new configuration against the last one to
   decide what changed and whom to wake, so an engine that answers
   differently on a second call reports changes that did not happen.

#### What the crate does around you

An engine never sees a file, an environment variable or a section, and never
needs to. By the time it is called, this crate has already: found and read
the documents, decrypted what was encrypted, narrowed each one to the
section being loaded, walked the environment and the `.env` files, read the
value strings, and put the layers in precedence order.

After it answers, this crate maps the tags back to origins, **fills in the
leaves the engine did not answer for**, applies aliases, narrows an
environment origin to the exact variable, and builds the snapshot.

That backfill is why rule 2's "no tags at all" is a real option. Under the
merge rule every engine implements, the winner of a leaf *is* the highest
layer that supplies that path — so the crate can read the answer off the
same input rather than guess at it, and provenance survives an engine that
does not track it.

#### Where an adapter usually goes wrong

Every one of these was a real bug in an adapter written here, caught by
the agreement tests rather than by review:

- **A key is a name, not a path.** `{"my.module": "debug"}` is one key with
  a dot in it. A backend that reads a key as a path expression turns it into
  `{"my": {"module": "debug"}}` — and a great deal of logging configuration
  is written exactly that way. Hand such a backend stand-in names and put
  the document's own back afterwards.
- **An empty table is a leaf.** `{"a": {}}` is a path a layer supplied,
  holding nothing. A backend that merges it by replacing the parent with a
  fresh empty map loses whose it was.
- **One integer, several widths.** This crate's tree holds every integer as
  an `i128`. A backend with a variant per width should be handed the
  narrowest that fits, or a round trip comes back a different width from the
  one it went in as.
- **Null is a value, not an absence.** A `null` in a document is a leaf that
  replaces what was under it. A backend that models it as "no key" turns a
  blanking somebody wrote on purpose into a no-op.

#### Proving it

The crate compares engines rather than trusting them, and the door is
`__fuzz::fold_through(engine, &layers)`: a stack of trees in, the folded
tree and the winner of every leaf out — the whole seam, with no filesystem.

Three corpora already run through every shipped engine and compare both the
tree and the provenance: the composition suite (`tests/composition.rs`),
generated layer stacks (the property test in `resolve.rs`), and the corner
cases above (`tests/engines.rs`). An engine written outside this crate can
drive the same door with its own cases.

### Adding an engine to this crate

Shipping an engine as an optional feature, in the order that fails fastest:

1. **The dependency**, optional, with `default-features = false` — a
   backend's own defaults are its idea of a sensible build, not this
   crate's. Then wire whichever of its features this crate's own map
   onto: a format feature here turns on the matching parser in every
   backend present, through cargo's `backend?/format` form, which fires
   only when the backend is already enabled. That is what keeps a second
   feature list from existing.

   ```toml
   my-backend = ["dep:my_backend"]
   json = ["dep:serde_json", "config_rs/json", "figment?/json", "my_backend?/json"]
   ```

   Check its MSRV against [this crate's](msrv-features.md) before
   anything else.

   Rename it if its crate name is a word this crate's macro uses as a helper
   attribute. `config` is renamed to `config_rs` for exactly that reason:
   with the plain name in scope, rustc appends "`config` is in scope, but it
   is a crate, not an attribute" to a diagnostic that was clear without it.

2. **The feature.** `my-engine = ["dep:my_backend"]`, and nothing else may
   enable it — a feature that drags an engine in makes it non-optional for
   everyone.

3. **The adapter**, as a directory under `src/backend/`:

   ```text
   src/backend/my_backend/
     mod.rs      what this backend is, and what it fills
     value.rs    between its value tree and this crate's
     reader.rs   its parsers, if it has any
     engine.rs   its fold, if it has one
     error.rs    its errors as this crate's, if they need translating
   ```

   **Grouped by backend rather than by seam**, which is the axis these
   change on: a backend's major release, or a decision to stop carrying
   one, touches one directory and nothing else. Add the constructor —
   `pub fn my_engine() -> &'static dyn Engine` — to `src/engine.rs`,
   beside the trait it implements. Read the four traps above first; each
   is a test you would otherwise fail.

4. **`engine::all()`.** This is the only list. The three corpora walk it, so
   an engine added here is compared against every other one without a test
   being edited — and an engine that nothing compares is an engine nobody
   has checked.

5. **The paperwork.** The table at the top of this page, the entry in
   [Cargo Features](features.md), a changelog line, and the graph cost said
   plainly: how many crates a default build gains, if the new engine is to
   be the default one.

Then `just check`. A disagreement shows up as the reference engine and the
new one printed side by side, on the case that separated them — from the
property test, the composition suite and the corner cases alike, since all
three walk the same list. Run the suite with `--no-fail-fast` to see every
case an engine broke rather than the first.

### Where the code lives

```text
src/engine.rs          the Engine trait and the registry
src/reader.rs          the Reader trait, this crate's own parsers, the registry
src/backend/
  config_rs/           value ─ reader ─ engine ─ error ─ source
  figment/             value ─ reader ─ engine ─ error ─ source
```

The seams — the traits, the one implementation this crate still owns (its
parsers), and the list a load picks from — stay beside the contract they
define. Everything
about one backend stays together. Outside `src/backend/`, no code in the
crate names a backend type except `Source::provider` — the one door where
a figment type is part of this crate's API — and the tests whose whole job
is comparing against a backend.

### What an engine costs

The `config` crate is not optional — it carries the fold, and there is no
other. It is taken with `default-features = false` and then given exactly
the parsers this crate's own format features name, so what it costs is
itself plus `pathdiff`: two crates, in every build.

Turning a format on adds that format's parser to whichever backends are
present, and where the two crates are the same one they share it: `toml`
is declared at the requirement `config` declares, so a TOML build has one
parser rather than a `0.9` beside a `1.x`. `yaml` is the case that does
not unify — `serde_yaml` for this crate's reader *and writer*,
`yaml-rust2` for the backend's reader.

## Readers

The **reader** is the step before everything else: text and a format in, a
[`Value`](https://docs.rs/dynamic-config/latest/dynamic_config/enum.Value.html)
out. Nothing above it sees a parser, which is what lets the parser be a
choice.

| reader | feature | parses |
|---|---|---|
| `native()` — **the default** | — | JSON, TOML, YAML, INI, `.properties` |
| `config_rs()` | — | JSON, TOML, YAML, INI, RON, JSON5 |
| `figment()` | `figment` | JSON, TOML, YAML |

**The column is what each one *parses*, not what a load that chose it can
read.** A format the chosen reader has no parser for is handed to one that
does, so choosing `config_rs()` for its YAML does not cost you the
`.properties` file beside it — see [below](#a-reader-that-cannot-read-a-format-hands-it-on).

```rust
let config: Db = DbConfig::builder("db")
    .file("config.yaml")
    .reader(dynamic_config::reader::config_rs())   // YAML through yaml-rust2
    .load()?;
```

Or once for the process, with `reader::set_reader(..)`.

Both seams are run side by side in
[`examples/engines.rs`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/engines.rs):
every engine folding the same layers to the same answer, every reader on
the same document, and the dotted-key corner below printed rather than
described.

### A reader that cannot read a format hands it on

The chosen reader gets first refusal; a format it does not read goes to
the first reader in `reader::all()` that does. So a `.ron` file works with
no reader installed at all, and `.properties` keeps working when the
backend's reader is chosen — the backend has no parser for it.

The fallback is **additive**: it can only fire where the chosen reader
would have failed outright, so a load that worked keeps working through
exactly the same parser it always did.

### Why the default reader is not the default engine

The engine defaults to `config-rs`; the reader defaults to this crate's
own. The asymmetry is about what can be proved.

A fold is **one rule** with an implementation on each side, and the tests
hold both to it leaf by leaf — so which engine runs is not a question
about what a configuration means. A parser is a **dialect**. Two YAML
libraries disagree about things no specification settles, this crate's INI
is the one [Formats](formats.md) specifies, and `.properties` has no
reader anywhere else. Handing those to a different library by default
would change what a document means for everyone who upgraded, quietly.

What the tests do hold is the part a deployment depends on: the shapes
documents actually take read the same through every reader, and **no
reader puts document content in an error** — the line that failed to parse
is, on a bad day, the line holding the password.

### Where the readers diverge

Recorded rather than smoothed over, and asserted in `tests/readers.rs` so
that a change to any of it is a test failure:

- **INI is two dialects.** This crate's own nests `[a.b]`, strips quotes,
  and leaves a `#` inside a value alone. The backend's is `rust-ini`, and
  it answers differently. A load that asks for the backend's reader gets
  the backend's INI.
- **`.properties` has one parser, and every reader reaches it.** Neither
  backend crate ships one — `config` reads six formats, figment three, and
  `.properties` is in neither list — so this crate's own parses it whoever
  was chosen, and `tests/readers.rs` loads the same document through every
  reader to keep that true. What a backend cannot do is read it
  *differently*: there is one properties dialect here, which is the
  [JDK's, deviations named](formats.md).
- **RON and JSON5 have no reader here.** They arrive with the `config-rs`
  reader, behind the `ron` and `json5` features, and are **read-only**:
  neither that crate nor this one has a writer, so `save()` refuses them
  the same way it refuses INI.

### What this buys, and what it costs

The reason to reach for a backend's reader is usually one of two:

- **YAML through a maintained parser.** This crate's own YAML is
  `serde_yaml`, which its author archived; the backend's is `yaml-rust2`,
  which is maintained.
- **A format this crate does not ship** — RON, JSON5.

The cost is the dialect question above, and — for YAML, the one format
where the two crates differ — a dependency graph that carries both
parsers: `features = ["yaml"]` compiles `serde_yaml` *and* `yaml-rust2`.
The first
is not there only for reading — this crate's `save()` writes YAML with it,
and neither backend has a writer at all — so the archived parser leaves a
build when YAML writing does, not when a different reader is chosen.

A format feature here — `json`, `toml`, `yaml`, `ini` — turns on the
matching parser in *whichever* backend is present, so there is no second
feature list to keep in step: `features = ["yaml"]` means this build reads
and writes YAML, and every reader in it can read it.
