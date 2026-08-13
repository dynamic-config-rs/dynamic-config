//! `dotenv::parse`, over generated `.env` text.
//!
//! What a crash means: a `.env` file next to a deployment takes the process
//! down at startup. The splitter runs before anything is validated — it is
//! the first code a `.env` reaches — and it does real slicing on every line:
//! a `trim`, a `strip_prefix`, a `split_once`, and a quote strip that takes
//! a byte off each end. Any of those is a panic on a string nobody wrote a
//! test for.
//!
//! Lines rather than bytes: a `.env` is a line grammar, and a generator that
//! spends its budget discovering `=`, `#`, `export ` and the quote pair never
//! reaches the branches under them. `Line::Raw` keeps arbitrary text
//! reachable, so the shapes nobody thought to name are still in the search.
//!
//! The properties:
//!
//! * parsing is total — every input is a map or an `Err`, never a panic;
//! * a rejection names a real line, and that line *alone* is rejected too:
//!   the number in the error is the line a user has to go and look at;
//! * the parser stops at the first bad line — everything above the reported
//!   one parses on its own;
//! * a name is never empty and never carries surrounding whitespace, because
//!   it becomes an environment variable name and ` DB_HOST` is not one;
//! * what parses, re-renders and parses back to the same thing.
//!
//! **The round trip skips three shapes on purpose**, and each is a decision
//! rather than a bug. A name beginning `export ` re-parses without it, because
//! `export` is a prefix this format accepts and cannot then distinguish from a
//! name. A name beginning `#` re-parses as a comment. A value that begins and
//! ends with a matching quote loses that pair, because stripping it is what
//! quoting is *for*. Asserting round-tripping over those would be asserting
//! that a documented rule is wrong.

#![no_main]

use std::collections::BTreeMap;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// One line of a `.env`, in the shapes the parser distinguishes.
#[derive(Arbitrary, Debug)]
enum Line {
    Blank,
    Comment(String),
    /// `export ` is accepted because a `.env` is very often also sourced by a
    /// shell, which makes it a branch worth reaching on purpose.
    Assignment {
        export: bool,
        name: String,
        value: String,
        quote: Option<Quote>,
    },
    /// Whatever the generator likes, including text that is not a line at all.
    Raw(String),
}

#[derive(Arbitrary, Debug)]
enum Quote {
    Single,
    Double,
}

impl Line {
    fn render(&self) -> String {
        match self {
            Line::Blank => String::new(),
            Line::Comment(text) => format!("#{text}"),
            Line::Assignment {
                export,
                name,
                value,
                quote,
            } => {
                let prefix = if *export { "export " } else { "" };

                let value = match quote {
                    Some(Quote::Single) => format!("'{value}'"),
                    Some(Quote::Double) => format!("\"{value}\""),
                    None => value.clone(),
                };

                format!("{prefix}{name}={value}")
            }
            Line::Raw(text) => text.clone(),
        }
    }
}

/// Whether `(name, value)` survives being written back out as `name=value`.
///
/// The three exclusions are the documented rules of the format, not defects;
/// see the module comment.
fn round_trips(name: &str, value: &str) -> bool {
    if name.starts_with("export ") || name.starts_with('#') {
        return false;
    }

    // Written back out, surrounding whitespace would be trimmed off again —
    // which is exactly why a value that wants to keep it has to be quoted.
    if value != value.trim() {
        return false;
    }

    // A value that already looks quoted would come back one pair shorter.
    let quoted = ['"', '\'']
        .into_iter()
        .any(|quote| value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote));

    !quoted
}

fuzz_target!(|lines: Vec<Line>| {
    let rendered: Vec<String> = lines.iter().map(Line::render).collect();
    let text = rendered.join("\n");

    let entries = match dynamic_config::__fuzz::dotenv_entries(&text) {
        Ok(entries) => entries,
        Err(line) => {
            // The number goes into an error a user reads next to their file,
            // so it has to name a line that is really there.
            let count = text.lines().count();

            assert!(
                line >= 1 && line <= count,
                "line {line} is not one of the {count} lines of the input"
            );

            let culprit = text.lines().nth(line - 1).expect("in range, just checked");

            // Attributable: whatever went wrong is a property of that line
            // and not of the ones around it.
            assert_eq!(
                dynamic_config::__fuzz::dotenv_entries(culprit),
                Err(1),
                "line {line} was blamed but parses on its own: {culprit:?}"
            );

            // And it is the *first* failure: everything above it was fine.
            let above: Vec<&str> = text.lines().take(line - 1).collect();

            assert!(
                dynamic_config::__fuzz::dotenv_entries(&above.join("\n")).is_ok(),
                "line {line} was blamed, but a line above it fails too"
            );

            return;
        }
    };

    let mut kept = BTreeMap::new();

    for (name, value) in &entries {
        assert!(!name.is_empty(), "an empty name would name no variable");
        assert_eq!(name.trim(), name, "a name keeps no surrounding whitespace");
        assert!(
            !name.contains('='),
            "a name stops at the first `=`: {name:?}"
        );

        if round_trips(name, value) {
            kept.insert(name.clone(), value.clone());
        }
    }

    // Re-rendered, the entries that survive the format's own rules parse back
    // to themselves. A splitter that dropped a byte off a name or kept one
    // too many on a value shows up here rather than as a variable that is
    // silently not set.
    let again: Vec<String> = kept
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();

    assert_eq!(
        dynamic_config::__fuzz::dotenv_entries(&again.join("\n")),
        Ok(kept.clone()),
        "re-parsing what was written back out changed it"
    );
});
