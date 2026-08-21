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

/// Every `.md` under `directory`, as paths relative to the repository.
fn collect_markdown(directory: &Path, into: &mut Vec<PathBuf>, repo: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();

        if path.is_dir() {
            collect_markdown(&path, into, repo);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            if let Ok(relative) = path.strip_prefix(repo) {
                into.push(relative.to_path_buf());
            }
        }
    }
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
    // Three crates are legitimately exempt: the CLI, the Python extension
    // and the Node addon are *installed* rather than depended on (`cargo
    // install`, `pip install`, `npm install`), and none of those install
    // lines carries a version by design.
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
        let exempt = rendered.contains("dynamic-config-cli")
            || rendered.contains("dynamic-config-python")
            || rendered.contains("dynamic-config-node");

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

/// Every number this repository's prose commits to, counted from the
/// workspace instead of remembered.
///
/// "Four crates", "four publish" — each is a claim with no compiler behind
/// it, and each is wrong the day a crate is added. This is that compiler,
/// and it has caught three: the ROADMAP said sixteen crates were on
/// crates.io when fourteen published, two pages said seven store crates
/// when git made it eight, and after the split every one of those numbers
/// described a workspace this repository no longer is.
///
/// Phrases rather than bare nouns, because "crates" alone means two
/// different numbers a sentence apart — *four crates in one workspace*,
/// *four crates on crates.io* — and a test that could not tell them apart
/// would have to be taught to ignore one of them.
///
/// The stores, the server and the bindings are counted by their own
/// repositories now. A claim here about *their* number is a claim this
/// workspace cannot check, so the prose does not make one.
#[test]
fn the_prose_counts_match_the_workspace() {
    let repo = repo();

    let Ok(entries) = fs::read_dir(&repo) else {
        eprintln!("skipped: not a repository checkout");
        return;
    };

    let mut members = 0_usize;
    let mut published = 0_usize;

    for entry in entries.filter_map(Result::ok) {
        let manifest = entry.path().join("Cargo.toml");
        let name = entry.file_name().to_string_lossy().to_string();

        if !name.starts_with("dynamic-config") || !manifest.exists() {
            continue;
        }

        let text = fs::read_to_string(&manifest).expect("a manifest is readable");

        members += 1;

        if !text.contains("publish = false") {
            published += 1;
        }
    }

    // One table, asked both questions: *what could a page be saying* — it
    // is the candidate list below — and *what does the workspace say*. Two
    // lists would drift, and did: the candidates used to stop at seventeen
    // while the workspace had eighteen members, so the two loudest claims
    // in the repo were the two this test could not see.
    const WORDS: [(usize, &str); 17] = [
        (4, "four"),
        (5, "five"),
        (6, "six"),
        (7, "seven"),
        (8, "eight"),
        (9, "nine"),
        (10, "ten"),
        (11, "eleven"),
        (12, "twelve"),
        (13, "thirteen"),
        (14, "fourteen"),
        (15, "fifteen"),
        (16, "sixteen"),
        (17, "seventeen"),
        (18, "eighteen"),
        (19, "nineteen"),
        (20, "twenty"),
    ];

    let word = |number: usize| -> &'static str {
        WORDS
            .iter()
            .find_map(|(value, spelled)| (*value == number).then_some(*spelled))
            .unwrap_or_else(|| panic!("no word for {number}; add it to WORDS, and check the prose"))
    };

    // Each phrase, and the number it is a claim about. A phrase that appears
    // nowhere is not an error — prose is allowed to not say a thing — but a
    // phrase that appears with the wrong number is.
    let claims: [(&str, usize); 6] = [
        ("crates in one workspace", members),
        ("crates share", members),
        // The first half of "Four crates. Four publish to crates.io": two
        // numbers in one sentence, matched by one row each.
        ("crates.", members),
        ("crates on crates.io", published),
        ("to crates.io", published),
        ("publish to crates.io", published),
    ];

    // The workspace's own numbers, spelled, before a single page is read.
    // This is what makes WORDS a gate rather than a lookup: a workspace
    // that grows past the table stops here, saying so, instead of quietly
    // checking prose against a number it has no word for.
    for (_, count) in claims {
        let _ = word(count);
    }

    // The five top-level documents, and every page of all three books —
    // which is where "the seven store crates" survived two releases after
    // git made it eight, because nothing counted them.
    let mut documents: Vec<PathBuf> = [
        "README.md",
        "ROADMAP.md",
        "RELEASING.md",
        "CONTRIBUTING.md",
        "AGENTS.md",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();

    for book in ["book/src", "book-python/src", "book-node/src"] {
        collect_markdown(&repo.join(book), &mut documents, &repo);
    }

    let mut wrong: Vec<String> = Vec::new();

    for name in &documents {
        let Some(text) = read(&repo.join(name)) else {
            continue;
        };

        let name = name.display();

        for (line_number, line) in text.lines().enumerate() {
            let lowered = line.to_lowercase();

            for (phrase, count) in claims {
                for (candidate, spelled) in WORDS {
                    let claim = format!("{spelled} {phrase}");

                    if lowered.contains(&claim) && candidate != count {
                        wrong.push(format!(
                            "{name}:{}: says \"{claim}\", and there are {count}",
                            line_number + 1
                        ));
                    }
                }
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "prose that disagrees with the workspace:\n  {}",
        wrong.join("\n  ")
    );
}

/// The precedence chain is written in two places, and they are the same
/// string.
///
/// The crate front page and the book's own chapter both draw it, because both
/// are somebody's first page. Two copies of an ordering is two chances to be
/// wrong about it, and one of them already was: `secrets_dir` landed in the
/// book's chain and never reached `lib.rs`, so the front page described a
/// layer order the loader had not had for two releases.
#[test]
fn the_precedence_chain_is_the_same_in_both_places() {
    let repo = repo();

    let (Some(front_page), Some(chapter)) = (
        read(&repo.join("dynamic-config/src/lib.rs")),
        read(&repo.join("book/src/sources-and-precedence.md")),
    ) else {
        eprintln!("skipped: not a repository checkout");
        return;
    };

    /// The `set_default < … < set_override` line, with the prose stripped
    /// off: `//! ` in Rust, nothing in Markdown.
    fn chain(text: &str) -> Option<String> {
        text.lines()
            .map(|line| line.trim_start_matches("//!").trim())
            .find(|line| line.starts_with("set_default <"))
            .map(str::to_owned)
    }

    let front = chain(&front_page).expect("lib.rs draws the chain");
    let book = chain(&chapter).expect("the chapter draws the chain");

    assert_eq!(
        front, book,
        "the crate front page and the book disagree about layer order; the \
         loader's own `LAYERS` table in `loader/mod.rs` is the tiebreak"
    );
}

/// Every example has a row in the book's table.
///
/// The table is how anybody finds them, and an example nobody can find is an
/// example nobody runs — which is the state `ini_provider` was in for two
/// releases, with the count above the table saying twenty-seven while
/// twenty-eight compiled.
#[test]
fn every_example_is_in_the_books_table() {
    let repo = repo();

    let (Ok(entries), Some(table)) = (
        fs::read_dir(repo.join("dynamic-config/examples")),
        read(&repo.join("book/src/examples.md")),
    ) else {
        eprintln!("skipped: not a repository checkout");
        return;
    };

    let mut missing: Vec<String> = Vec::new();
    let mut examples = 0_usize;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();

        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("an example has a name")
            .to_owned();

        examples += 1;

        // The row links to the file, so the path is the thing to look for:
        // a name alone would match this example being *mentioned* in another
        // row's prose.
        if !table.contains(&format!("examples/{stem}.rs")) {
            missing.push(stem);
        }
    }

    assert!(
        missing.is_empty(),
        "examples with no row in book/src/examples.md: {missing:?}"
    );

    // And the count in the sentence above the table.
    let counted = match examples {
        26 => "twenty-six",
        27 => "twenty-seven",
        28 => "twenty-eight",
        29 => "twenty-nine",
        30 => "thirty",
        31 => "thirty-one",
        other => panic!("no word for {other} examples; add it, and check the prose"),
    };

    assert!(
        table.to_lowercase().contains(&format!("{counted} of them")),
        "the book says something other than \"{counted} of them\", and there \
         are {examples}"
    );
}
