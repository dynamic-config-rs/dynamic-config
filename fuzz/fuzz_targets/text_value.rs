//! The reading of configuration text, over arbitrary input.
//!
//! What a crash means: an environment variable takes the process down at
//! load. That is not hypothetical — the implementation this grammar was
//! ported from indexes a byte range with a character position while
//! unescaping, so `APP_GREETING=\'"é\\n"\'` aborted every load that saw it.
//! This target exists so the port cannot acquire a fault of its own.
//!
//! The property is total-ness: every string is a value. Not "parses or
//! errors" — there is no error to return, because text the grammar cannot
//! read is text.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let read = dynamic_config::__fuzz::text_value(text);

    assert!(
        !read.is_empty(),
        "every string has a reading, even the empty one"
    );
});
