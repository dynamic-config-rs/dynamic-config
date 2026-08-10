//! A macro error must not swallow the struct: the type still exists, so the
//! only error is the macro's own — no "cannot find type" cascade.

#[dynamic_config::dynamic_config(files = ["config.json"], key = "db", wrong_argument)]
#[derive(Debug, serde::Deserialize)]
struct StillExists {
    value: u32,
}

fn takes_it(_: &StillExists) {}

fn main() {
    // The struct itself remains usable even though the attribute failed.
    let _ = takes_it;
}
