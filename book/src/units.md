# Units

`timeout = 30` is ambiguous and `max_body = 67108864` is unreadable, so both are
usually written with a unit — which no stock `Deserialize` accepts:

```rust
#[derive(Deserialize)]
struct Limits {
    #[serde(with = "dynamic_config::duration")]
    timeout: Duration,      // "30s", "1h30m", "500ms", or a number of seconds
    #[serde(default, with = "dynamic_config::duration::option")]
    grace: Option<Duration>,
    #[serde(with = "dynamic_config::bytes")]
    max_body: u64,          // "64MiB", "1GB", or a number of bytes
}
```

`KiB`/`MiB`/`GiB` are powers of 1024, `KB`/`MB`/`GB` powers of 1000, and a bare
`K`/`M`/`G` is read as the binary form. An unknown unit is an error listing the
valid ones, never a silent zero.
