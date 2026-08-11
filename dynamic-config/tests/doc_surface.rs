//! The documented surface matches the generated one.
//!
//! The macro's method list is written down twice outside the macro itself: the
//! book's attribute reference and the crate front page. Both went stale once —
//! methods renamed, signatures changed, rows never added — and nothing noticed,
//! because prose has no compiler. This test is that compiler: it derives the
//! generated method names from the macro's own source and diffs them against
//! both documents.
//!
//! The test reads sibling files from the repository checkout. In a published
//! package those siblings do not ship, so it skips itself there — CI is where
//! it gates.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every `pub fn` / `pub async fn` name in the given source text.
///
/// In the macro crate these only occur inside `quote!` blocks — the crate's own
/// helpers are `pub(crate)` or tighter — so the matches *are* the generated
/// surface.
fn generated_names(source: &str, into: &mut BTreeSet<String>) {
    for line in source.lines() {
        let trimmed = line.trim_start();

        for prefix in ["pub fn ", "pub async fn "] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                    .collect();

                if !name.is_empty() {
                    into.insert(name);
                }
            }
        }
    }
}

/// Whether `name` occurs in `text` as a whole word — `load` in `load_async`
/// does not count.
fn mentions(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();

    text.match_indices(name).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i] as char);
        let after = bytes.get(at + name.len()).map(|b| *b as char);
        let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');

        !is_word(before) && !is_word(after)
    })
}

/// Method names from the first column of a markdown table's rows.
///
/// Two spellings appear: `name(args)` and a bare backticked `name`. Everything
/// else in the cell — types, `&self`, arrows — stays out because it is either
/// not followed by `(` or not the whole backticked token.
fn documented_names(table_lines: &[&str]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();

    for line in table_lines {
        let Some(cell) = line.trim_start_matches('|').split('|').next() else {
            continue;
        };

        if !cell.contains('`') {
            continue; // the header row and the separator
        }

        // `name(` anywhere in the cell. `rfind` stops at the first character
        // that cannot be part of a name, so `start..at` is the whole name.
        for (at, _) in cell.match_indices('(') {
            let start = cell[..at]
                .rfind(|c: char| !(c.is_ascii_lowercase() || c == '_'))
                .map_or(0, |i| i + 1);

            if start < at {
                names.insert(cell[start..at].to_string());
            }
        }

        // A backtick span that is nothing but a name.
        for span in cell.split('`').skip(1).step_by(2) {
            if !span.is_empty() && span.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                names.insert(span.to_string());
            }
        }
    }

    names
}

/// The lines of the "What the attribute generates" section, table rows only.
fn generates_section(text: &str, heading: &str) -> Vec<String> {
    let mut inside = false;
    let mut rows = Vec::new();

    for line in text.lines() {
        let line = line.trim_start_matches("//!").trim_start();

        if line.starts_with(heading) {
            inside = line.contains("What the attribute generates");
        } else if inside && line.starts_with('|') {
            rows.push(line.to_string());
        }
    }

    rows
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[test]
fn the_documented_methods_are_the_generated_ones() {
    let repo = repo();
    let book = repo.join("book/src/attribute-reference.md");
    let expand = repo.join("dynamic-config-macros/src/expand");

    let (Some(book), Ok(entries)) = (read(&book), fs::read_dir(&expand)) else {
        eprintln!("skipped: not a repository checkout");
        return;
    };

    let mut generated = BTreeSet::new();

    for entry in entries {
        let path = entry.expect("the directory listing is readable").path();

        if path.extension().is_some_and(|e| e == "rs") {
            generated_names(
                &fs::read_to_string(&path).expect("a source file is readable"),
                &mut generated,
            );
        }
    }
    for entry in
        fs::read_dir(repo.join("dynamic-config/src/redirects")).expect("src/redirects/ is readable")
    {
        let path = entry.expect("the directory listing is readable").path();

        if path.extension().is_some_and(|e| e == "rs") {
            generated_names(
                &fs::read_to_string(&path).expect("a redirect file is readable"),
                &mut generated,
            );
        }
    }

    assert!(
        generated.contains("builder") && generated.len() > 30,
        "the extraction found {} names — the macro layout moved and this \
         test's paths need to follow it",
        generated.len()
    );

    // Completeness: every generated method is somewhere in the book's
    // attribute reference.
    let missing: Vec<_> = generated
        .iter()
        .filter(|name| !mentions(&book, name))
        .collect();

    assert!(
        missing.is_empty(),
        "generated but absent from book/src/attribute-reference.md: {missing:?}"
    );

    // No staleness: every method the two tables name is really generated.
    let lib =
        fs::read_to_string(repo.join("dynamic-config/src/lib.rs")).expect("lib.rs is readable");

    for (place, rows) in [
        (
            "book/src/attribute-reference.md",
            generates_section(&book, "## "),
        ),
        ("dynamic-config/src/lib.rs", generates_section(&lib, "# ")),
    ] {
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let documented = documented_names(&rows);
        let stale: Vec<_> = documented.difference(&generated).collect();

        assert!(
            !documented.is_empty(),
            "{place}: found no method names — the section heading moved"
        );
        assert!(
            stale.is_empty(),
            "{place} documents methods the macro does not generate: {stale:?}"
        );
    }
}
