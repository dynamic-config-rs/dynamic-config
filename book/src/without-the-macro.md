# Without the macro

The engine is public and usable on its own:

```rust
use dynamic_config::{load, ConfigCell, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Deserialize)]
struct Db { host: String }

static DB: ConfigCell<Db> = ConfigCell::new();

let sources = [Source::inline(r#"{"db": {"host": "localhost"}}"#, Format::Json)];
let db: Db = load(&LoadSpec { key: "db", sources: &sources, env_prefix: None })?;

DB.store(db);
assert_eq!(DB.load().unwrap().host, "localhost");
# Ok::<(), dynamic_config::Error>(())
```
