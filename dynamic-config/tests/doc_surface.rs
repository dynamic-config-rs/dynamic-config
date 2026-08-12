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

/// Every README's install snippet names the same version.
///
/// The workspace releases in lockstep, and the failure mode is always the
/// same: the root README gets updated for a release and a companion's
/// snippet keeps quoting the version before it — which is exactly what
/// happened between 0.2.0 and 0.3.0, nine files at a time. Consistency is
/// the honest invariant to pin (rather than equality with `Cargo.toml`,
/// which is legitimately one commit ahead mid-release): all snippets move
/// together, or the gate names the stragglers.
#[test]
fn the_readmes_agree_on_one_version() {
    let repo = repo();

    let Ok(entries) = fs::read_dir(&repo) else {
        eprintln!("skipped: not a repository checkout");
        return;
    };

    let mut readmes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("dynamic-config"))
        })
        .map(|path| path.join("README.md"))
        .filter(|path| path.exists())
        .collect();
    readmes.push(repo.join("README.md"));

    let mut versions: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    // A README that contributes *nothing* is the exact regression this gate
    // exists for — a snippet deleted, or rewritten into a shape the parser
    // no longer sees — so per-file accounting is part of the assertion.
    // Two crates are legitimately exempt: the CLI and the Python
    // extension are *installed* rather than depended on (`cargo install`,
    // `pip install`), and neither install line carries a version by
    // design.
    let mut empty: Vec<String> = Vec::new();

    for readme in &readmes {
        let text = fs::read_to_string(readme).expect("a README is readable");
        let before: usize = versions.values().map(Vec::len).sum();

        // `dynamic-config… = "X.Y.Z"` and `version = "X.Y.Z"`, inside toml
        // fences; `<version>` placeholders (the book's convention) and the
        // exact-pin `=X.Y.Z` internal form are somebody else's business.
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };

            if !name.trim_start().starts_with("dynamic-config") && name.trim() != "version" {
                continue;
            }

            for candidate in value.split('"').skip(1).step_by(2) {
                let plausible = candidate.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && candidate.split('.').count() == 3;

                if plausible {
                    versions
                        .entry(candidate.to_string())
                        .or_default()
                        .push(readme.display().to_string());
                }
            }
        }

        let contributed = versions.values().map(Vec::len).sum::<usize>() > before;
        let rendered = readme.display().to_string();
        let exempt =
            rendered.contains("dynamic-config-cli") || rendered.contains("dynamic-config-python");

        if !contributed && !exempt {
            empty.push(readme.display().to_string());
        }
    }

    assert!(
        empty.is_empty(),
        "these READMEs contribute no install-snippet version — deleted \
         snippet, or a shape the parser no longer sees: {empty:?}"
    );
    assert!(
        !versions.is_empty(),
        "found no install-snippet versions — the extraction broke"
    );
    assert_eq!(
        versions.len(),
        1,
        "the READMEs disagree on the release version: {versions:#?}"
    );
}
