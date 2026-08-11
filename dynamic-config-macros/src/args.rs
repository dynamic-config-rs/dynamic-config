//! Attribute input: there is none.
//!
//! The attribute declares — *this type is a configuration* — and the builder
//! configures. Every source argument the attribute used to take lives on
//! [`Builder`](https://docs.rs/dynamic-config/latest/dynamic_config/struct.Builder.html)
//! now, chosen at runtime where sources actually vary. What remains here is
//! the part a builder cannot do: generate the type's storage and surface.
//!
//! Anything between the parentheses is rejected with directions, not a
//! shrug: the migration is mechanical, and the error message is the map.

use syn::parse::{Parse, ParseStream};
use syn::Result;

/// The (empty) attribute input.
pub(crate) struct Args;

impl Parse for Args {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self);
        }

        Err(input.error(
            "#[dynamic_config] takes no arguments since 0.2: the attribute \
             declares, the builder configures. Move the source arguments to \
             the builder — `files`/`name`+`paths` become `.file(..)` / \
             `.discover(name, paths)`, `key` moves to `builder(\"key\")`, \
             `env`/`nest`/`allow_empty_env`/`strict_env` become `.env(..)`, \
             `.nest(..)`, `.allow_empty_env()`, `.strict_env()`, `env_files` \
             becomes `.env_file(..)`, `profile_env` becomes \
             `.profile_env(..)`, `cache`/`cache_mode` become `.cache(path, \
             mode)`, `validate` becomes `.validate(f)`, and `watch`/`poll` \
             become `.watch(debounce)` / `.watch_with(debounce, mode)` on \
             the handle `init()` was called on. `save` and `schema` are the \
             free functions `dynamic_config::save` and \
             `dynamic_config::schema::document`.",
        ))
    }
}
