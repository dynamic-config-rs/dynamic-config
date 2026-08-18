//! The two flat-format parsers, on arbitrary text.
//!
//! Hand-written parsers over untrusted files: exactly what a fuzzer is
//! for. The property is the parsers' whole contract — any input either
//! parses or answers an error, and neither panics, loops, nor overflows.
//! (That errors carry no document content is asserted by the redaction
//! target's rules; here the fuzzer only drives.)

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = dynamic_config::__fuzz::ini_document(text);
    let _ = dynamic_config::__fuzz::properties_document(text);
});
