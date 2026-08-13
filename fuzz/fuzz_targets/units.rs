//! `duration::parse` and `bytes::parse`, over arbitrary text.
//!
//! What a crash means: a configuration *value* — `timeout = "..."`,
//! `max_body = "..."` — takes a load down. These parsers do real arithmetic
//! on attacker-shaped input (multiplications for `h` and `d`, for `MB` and
//! `GB`) and real slicing on it, which is two ways to panic on a string
//! nobody thought about.
//!
//! Both are total functions: every input is either a value or an `Err`, and
//! neither may panic. That is the whole property.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    // The `Err` string is part of the surface too: it is built with
    // `format!` over slices of the input, so a bad boundary shows up here
    // rather than in the happy path.
    if let Err(reason) = dynamic_config::duration::parse(text) {
        assert!(!reason.is_empty(), "a rejection must say why");
    }

    if let Err(reason) = dynamic_config::bytes::parse(text) {
        assert!(!reason.is_empty(), "a rejection must say why");
    }
});
