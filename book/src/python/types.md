# Data Types

Whatever Pydantic can validate, this can load. The binding converts the
resolved tree into plain Python data — dicts, lists, strings, numbers,
booleans, `None` — and Pydantic does the rest, which means every
coercion, validator and custom type you already use keeps working.

## What arrives, and as what

```python
class Everything(BaseModel):
    text: str
    count: int
    ratio: float
    enabled: bool
    nothing: Optional[str] = None

    level: Level                      # a str or int Enum
    mode: Literal["read", "write"]

    started: datetime                 # from an ISO string
    day: date
    window: timedelta                 # from seconds, or a duration string
    identifier: UUID
    amount: Decimal
    address: IPv4Address
    where: Path

    names: list[str]
    ports: set[int]
    pair: tuple[str, int]
    labels: dict[str, str]

    primary: Endpoint                 # a nested model
    replicas: list[Endpoint]
    by_region: dict[str, Endpoint]
    either: Union[int, str]
    secret: SecretStr
```

All of it is covered by the test suite against real TOML, JSON and
environment sources — including the coercions Pydantic performs from
strings, which is how a `UUID` or a `datetime` gets into a config file in
the first place.

## The conversions that have to be exact

Three of them are silent when they go wrong, so they get their own tests:

**An integer stays an integer.** `port = 5432` must not arrive as
`5432.0`. The conversion builds Python ints from the loader's integers
rather than routing everything through a float, and there is no JSON
string round trip anywhere in the path.

**A boolean stays a boolean.** In Python `bool` is a subclass of `int`,
so a careless conversion turns `True` into `1` — and back into `True`
often enough that nobody notices until a `Literal[1]` or a `is True`
somewhere disagrees.

**Large integers keep their digits.** A `u64` above `i64::MAX` is a real
configuration value (a nanosecond timestamp, a snowflake id). It arrives
as a Python int, not a float that has quietly dropped the last three
digits.

## Values you supply from Python

The runtime layers take Python objects and put them where a file's values
go, so the conversion runs the other way:

```python
config.set_default("pool_size", 8)
config.set_default("labels", {"team": "platform"})
config.set_defaults(Database(host="fallback"))     # a whole model
config.set_override("host", "localhost")           # outranks everything
```

Anything without a configuration meaning is refused at the call rather
than serialized into something surprising: a function, an open file, a
`NaN`, a dict with non-string keys. `SecretStr` and `SecretBytes` are
understood — their value is taken, because you are supplying it — as are
Pydantic models and anything else with `model_dump`.

## Environment variables and types

The environment is strings, and figment's loose parsing turns them into
what the field wants: `APP_DB_PORT=5432` reaches an `int`,
`APP_DB_ENABLED=true` a `bool`. Loose parsing is ergonomic and ambiguous
at the edges — `off` reads like a boolean and arrives as the string
`"off"` — so `strict_env()` refuses that family with an error naming the
variable. Nesting uses a doubled separator:
`APP_DB_POOL__MAX_SIZE` sets `pool.max_size`.

## What the boundary will not do

- **`bytes` fields** work through Pydantic's own coercion from a string;
  the loader has no binary literal, because no configuration format this
  crate reads has one.
- **Arbitrary Python objects** cannot be a configuration value. If a
  model field is a type Pydantic can build *from* a string, a number or a
  mapping, it works; if it can only be built by running Python, the value
  belongs in code rather than in a config file.

## What a schema may be

Four kinds of class can be a schema — and a fifth answer, which is *no
schema*. The whole surface — sources, precedence, watching, recovery,
diagnostics — is identical across all of them. What differs is what
*validation* means, and what you have to install:

| Schema | Install | Validation |
|---|---|---|
| `dataclasses.dataclass` | nothing | structural: required fields, unknown keys, nested dataclasses, and each value against its declared type |
| `pydantic.BaseModel` | `[pydantic]` | Pydantic's, entire — coercion, constraints, validators, computed fields |
| `pydantic.dataclasses.dataclass` | `[pydantic]` | the same, through the dataclass validator |
| `pydantic_settings.BaseSettings` | `[pydantic-settings]` | Pydantic's, plus a [sourcing declaration this engine can translate](#pydantic-settings) |
| `Values` | nothing | none — [a configuration with no schema](#values-a-configuration-with-no-schema) |

The base install has no dependencies at all; each extra buys one more
kind of schema and nothing else. `[all]` is both.

### `Values`: a configuration with no schema

The Python spelling of the crate's [schemaless
configuration](../schemaless.md), for the keys a program learns at run
time rather than declares — a plugin host, a feature-flag table, a tool
reading a file it did not write. Pass the **class**; every load hands
back an **instance**:

```python
from dynamic_config import DynamicConfig, Values

config = DynamicConfig(Values, key="plugins").file("plugins.toml").env("APP_")
config.init()

values = config.current()

values["cache.ttl"]           # by dotted path
values.get("cache.ttl", 60)   # ...with a default
values["cache"]["ttl"]        # or a step at a time
dict(values)                  # a plain dict
```

It is a `Mapping`, so `len()`, `in`, `.keys()`, `.items()` and iteration
work as they do on a dict, and every value is already a plain Python
object — `str`, `int`, `float`, `bool`, `list`, `dict`, `None`. There is
nothing to unwrap. Lookup takes a **dotted path**, which is the one place
it is not a dict: a key that itself contains a dot is not reachable by
name, the same trade the Rust `Value::get` makes.

Everything else is the engine you already have: the same layers and
precedence, profiles, discovery, the secrets directory, the watcher,
reload hooks, `source_of`, `explain`, `snapshot` and `check`.

**What it gives up is exactly what it never declared.** Two answers
change, and both are reported rather than assumed:

| | A declared model | `Values` |
|---|---|---|
| `check()` unknown keys | compared against the field names | nothing to compare — `report.unknown_checked` is `False` and the rendering says `unknown keys: not checked (no field list)` |
| secret paths | derived from the declaration (`SecretStr`, `metadata={"secret": True}`) | **none**, unless `DynamicConfig(Values, key=…, secrets=["token"])` says so |

The second has teeth: a `redacted` or `fingerprint` [cache](reference.md)
is *refused* for a configuration that never said what is secret, rather
than writing a file that claims a redaction it did not perform. Naming
the paths with `secrets=` buys the cache and the `***` in `explain`
together.

`examples/20_schemaless.py` runs all of it.

### A dataclass, and what it checks

```python
from dataclasses import dataclass, field

@dataclass
class Database:
    host: str = "localhost"
    port: int = 5432
    password: str = field(default="", metadata={"secret": True})
```

`metadata={"secret": True}` is the stdlib's own extension point, and the
natural place for a declaration Pydantic makes with a type — `SecretStr`
has no stdlib equivalent, `metadata` does. It drives the same redaction:
the cache drops it, `explain` renders it `***`.

What the adapter checks:

- every **required** field present, and every key one the class declares;
- **nested dataclasses** built recursively rather than left as dicts;
- each value against its **declared type**, with `bool` and `int` kept
  apart in both directions, and `int` widening to `float` but not back.

What it does *not* do is coerce, with three deliberate exceptions where
the type parses its own text: an `Enum` takes its member's value,
`date`/`time`/`datetime` go through `fromisoformat`, and a type that
builds from a single argument is built from it (`UUID`, `Path`,
`Decimal`, `IPv4Address`). Anything else that does not match its
annotation is a validation failure naming the field and the two type
names — never the value, because that message travels into diagnostics.

One limitation is worth knowing, and it is Python's rather than this
library's: annotations are resolved with `typing.get_type_hints`, which
looks in the module where the class was defined. A dataclass declared
*inside a function* names types that module cannot see, so its
annotations stay strings and there is nothing to check them against.
Declare configuration dataclasses at module level. (Pydantic meets the
same wall and answers it with `model_rebuild()`.)

### What a Pydantic model may be

Everything, which is the reason to reach for one. All of these work, and
each has a test:

| Shape | Notes |
|---|---|
| **Inheritance**, any depth | Fields accumulate; a subclass may narrow a parent's field |
| **Mixins** beside `BaseModel` | An ordinary class in the bases is untouched |
| `model_config` | `extra` (`ignore`/`allow`/`forbid`), `frozen`, `populate_by_name`, `alias_generator`, `validate_assignment` |
| **Validators** | `field_validator` (both modes), `model_validator`; a rejection is a refused reload, never a half-installed one |
| **Computed fields** | Available on the installed model, like any property |
| **Private attributes** | `PrivateAttr` survives a load; it is not configuration |
| **`RootModel`** | As a field, and as the outer model |
| **Pydantic dataclasses** | As a field, secrets inside them included |
| **Generic models** | `Box[int]` directly, or a subclass that specialises it |
| **Discriminated unions** | The discriminator picks the member, as usual |
| **`BaseSettings`** | See below — it is a model, plus a thing this replaces |

### Aliases, in all four shapes

Pydantic accepts four alias declarations, and a field is reachable by all
of them at once: a plain string, an `AliasPath` into nested data, an
`AliasChoices` of either, and an `alias_generator` that writes them for
you. Each names what a *file* would carry, so each is what this binding
has to look for.

Two places depend on getting that right. The unknown-key report must not
call an accepted spelling unknown. And the redaction list — derived from
`SecretStr`/`SecretBytes` on the model — must hold **every** name the
field could arrive under, not one of them:

```python
class Credentials(BaseModel):
    password: SecretStr = Field(
        default=SecretStr(""),
        validation_alias=AliasChoices("password", "pass"),
    )
```

A file writing `pass` is writing a secret. `secret_paths(Credentials)`
answers `["password", "pass"]`, and both are redacted in `explain` and
dropped from the redacted cache. Listing a name nothing supplies costs a
key that never appears; missing one puts a password in a diagnostic and
on disk.

## pydantic-settings

`BaseSettings` is two things bolted together: a Pydantic model, and a set
of places to read it from. The first half works here unchanged — it is a
`BaseModel`. The second half is what this engine does instead, and it
does not run under `model_validate`, which is how this binding validates.
A class declaring `env_prefix` would therefore get none of it.

Silently, which is the part worth fixing. So:

```python
from dynamic_config import DynamicConfig

config = DynamicConfig.from_settings(ServiceSettings, key="svc")
config.init()
```

`from_settings` reads the class's `SettingsConfigDict` and rebuilds the
declaration as engine sources:

| Declared | Becomes |
|---|---|
| `toml_file`, `json_file`, `yaml_file` | `file(...)`, in that order |
| `env_file` | `env_file(...)` — the dotenv layer |
| `env_prefix` | one `bind_env` per leaf field, so `APP_PORT` stays `APP_PORT` rather than becoming `APP_<KEY>_PORT` |
| `secrets_dir` | `secrets_dir(...)` — a directory of single-value files |
| `env_nested_delimiter` | the separator inside those names |
| `case_sensitive` | whether they are upper-cased |

Precedence is this crate's, which agrees with pydantic-settings where the
two overlap: files lose to `.env`, which loses to the environment, which
loses to overrides. Bindings see `.env` files too, so a variable a
deployment writes into `.env` rather than exporting still reaches the
field it names.

`secrets_dir` translates too, onto the engine source of the same shape —
a directory where each file is one key.

What has no engine equivalent is **refused at the call** rather than
dropped: `cli_parse_args`, and an overridden
`settings_customise_sources`. Declare those on the configuration
instead — or keep the class for its schema and use `DynamicConfig`
directly, which is a fine thing to want:

```python
config = DynamicConfig(ServiceSettings, key="svc").file("service.toml")
```

That path warns if the class declares sourcing, and carries on. The
warning is not disapproval; it is the difference between choosing to be
the source and believing an `env_prefix` is doing something.

One difference in the schema half surprises people: `BaseSettings`
defaults to `extra="forbid"` where `BaseModel` ignores what it does not
declare. A narrow settings class pointed at a section carrying more than
it declares fails validation rather than shrugging.
