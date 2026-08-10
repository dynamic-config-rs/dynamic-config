# Generic configuration types

`Config<Postgres>` and `Config<Mysql>` are different types, so they get
different snapshots:

```rust
#[dynamic_config(files = ["config.toml"], key = "db")]
#[derive(Debug, Deserialize)]
struct Db<D: Driver> {
    url: String,
    #[serde(skip)]
    driver: PhantomData<fn() -> D>,   // `fn() -> D`, so the marker stays Send + Sync
}

Db::<Postgres>::init()?;
Db::<Mysql>::init()?;                 // its own snapshot, its own layers
```

Type and const parameters both work. A **lifetime** parameter does not, and is
rejected at compile time: the snapshot outlives every borrow that could name
one.

## It is not free, so you only pay for it if you use it

Rust has no generic statics, so a generic type's snapshot cannot live in one. It
goes through a `TypeId`-keyed registry instead. Measured on this machine with
`cargo bench -p dynamic-config --features json`, 5M reads each:

| Storage | `current()` |
|---|---|
| `static ConfigCell` (non-generic) | **17 ns** |
| `TypeId` registry (generic) | **27 ns** |

The macro knows which shape it is emitting, so a non-generic config type keeps
its `static` and its 17 ns — adding generic support cost existing code nothing.
The registry read is lock-free (an `ArcSwap` of the table, and `TypeId` passed
through rather than hashed); the first naive version, with an `RwLock` and
SipHash, measured 64 ns.

Either figure is noise next to a request. Both are the cost of *taking* a
snapshot, not of reading fields from one — take it once per unit of work and the
question stops mattering.
