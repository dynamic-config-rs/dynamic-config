//! Every rejection the argument parser can produce, one struct each.

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["a.json"], key = "a", bogus = 1)]
#[derive(Deserialize)]
struct UnknownArgument {
    x: u8,
}

#[dynamic_config(files = ["a.ini"], key = "a")]
#[derive(Deserialize)]
struct UnsupportedExtension {
    x: u8,
}

#[dynamic_config(files = ["noextension"], key = "a")]
#[derive(Deserialize)]
struct NoExtension {
    x: u8,
}

#[dynamic_config(files = ["a.json"], key = "a", debounce = 10)]
#[derive(Deserialize)]
struct DebounceWithoutWatch {
    x: u8,
}

#[dynamic_config(files = ["a.json"], key = "a", watch, debounce = 0)]
#[derive(Deserialize)]
struct ZeroDebounce {
    x: u8,
}

#[dynamic_config(files = [], key = "a")]
#[derive(Deserialize)]
struct EmptyFiles {
    x: u8,
}

#[dynamic_config(files = ["a.json"])]
#[derive(Deserialize)]
struct NoKey {
    x: u8,
}

#[dynamic_config(files = ["a.json"], key = "")]
#[derive(Deserialize)]
struct EmptyKey {
    x: u8,
}

#[dynamic_config(files = ["a.json"], files = ["b.json"], key = "a")]
#[derive(Deserialize)]
struct DuplicateFiles {
    x: u8,
}

#[dynamic_config(files = ["a.json"], key = "a", allow_empty_env)]
#[derive(Deserialize)]
struct EmptyEnvWithoutEnv {
    x: u8,
}

#[dynamic_config(name = "config", key = "a")]
#[derive(Deserialize)]
struct NameWithoutPaths {
    x: u8,
}

#[dynamic_config(paths = ["/etc/app"], key = "a")]
#[derive(Deserialize)]
struct PathsWithoutName {
    x: u8,
}

#[dynamic_config(key = "a")]
#[derive(Deserialize)]
struct NothingToLoad {
    x: u8,
}

#[dynamic_config(files = ["a.json"], key = "a", nest = "::")]
#[derive(Deserialize)]
struct NestWithoutEnv {
    x: u8,
}

// Type and const parameters are supported; a lifetime cannot be, because the
// snapshot outlives every borrow that could name one.
#[dynamic_config(files = ["a.json"], key = "a")]
#[derive(Deserialize)]
struct Borrowed<'a> {
    x: &'a str,
}

fn main() {}
