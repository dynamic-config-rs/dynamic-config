//! A dropped `HookGuard` is a hook that never fires.
//!
//! `on_reload_scoped` registers and hands back the guard that keeps the
//! registration alive; discarding it at the semicolon unregisters
//! immediately, so the callback is registered and gone before the next line
//! runs. Nothing about the program's behaviour says so — it simply never
//! fires — which is why the type is `#[must_use]` and why that is pinned
//! here.
//!
//! `#![deny(unused_must_use)]` because a user's crate warns where this test
//! has to fail.

#![deny(unused_must_use)]

use dynamic_config::{Builder, Dynamic};
use serde::Deserialize;

#[derive(Deserialize)]
struct Tenant {
    #[allow(dead_code)]
    name: String,
}

fn main() {
    let instance = Dynamic::new(Builder::<Tenant>::new("tenant"));

    instance.on_reload_scoped(|_, _| {});
    instance.on_reload_with_scoped(|_| {});
}
