//! Last-known-good, as a property: however a refused document is shaped,
//! the previous snapshot keeps serving, byte for byte — the Compatibility
//! Contract's third guarantee.
//!
//! The cell is the unit under test: install a generated document, then
//! throw generated garbage at the load path and record the refusal the way
//! every real reload path does (`Installer::record_failure` funnels here).
//! Whatever the garbage, `load()` answers with the installed document, the
//! published generation holds still, and the monotonic refusal counter is
//! the only thing that moved.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use dynamic_config::{load, ConfigCell, Format, LoadSpec, Source};

#[derive(Arbitrary, Debug)]
struct Input {
    /// The good document's single value — shape is irrelevant to the law.
    good: u32,
    /// Arbitrary bytes offered to the JSON parser as the "new" document.
    garbage: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let cell: ConfigCell<u32> = ConfigCell::new();

    cell.store(input.good);

    let generation = cell.generation();
    let refusals = cell.refusals();

    // A reload attempt over the garbage: parse-or-refuse through the real
    // loader, never a hand-rolled error.
    let text = String::from_utf8_lossy(&input.garbage).into_owned();
    let sources = [Source::inline(&text, Format::Json)];
    let spec = LoadSpec::new("doc", &sources).with_whole_document(true);

    match load::<u32>(&spec) {
        Ok(_) => {
            // The garbage happened to be a valid document; nothing to
            // assert about refusal, and installing it is not this
            // property's business.
        }
        Err(error) => {
            cell.record_failure(&error);

            let served = cell.load().expect("the previous snapshot serves");

            assert_eq!(*served, input.good, "LKG must serve A, unchanged");
            assert_eq!(
                cell.generation(),
                generation,
                "a refusal must not move the published generation"
            );
            assert_eq!(
                cell.refusals(),
                refusals + 1,
                "and must move the monotonic counter by exactly one"
            );

            // The error a refusal records never carries the document.
            let rendered = format!("{error}");
            if let Ok(garbage_text) = std::str::from_utf8(&input.garbage) {
                if garbage_text.len() > 8 {
                    assert!(
                        !rendered.contains(garbage_text),
                        "a refusal must not quote the refused document"
                    );
                }
            }
        }
    }
});
