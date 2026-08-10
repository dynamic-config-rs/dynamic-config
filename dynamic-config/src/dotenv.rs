//! A `.env` file, read as the environment rather than as a document.
//!
//! A `.env` holds *variable names*, not key paths:
//!
//! ```text
//! APP_DB_HOST=localhost
//! APP_DB_POOL__MAX_SIZE=32
//! ```
//!
//! So it is not another format for [`files`](crate::LoadSpec::sources) — the
//! same bytes in a `.json` would mean nothing. It is the environment layer,
//! sourced from disk, and it goes through exactly the same prefix stripping and
//! nesting as the real thing.
//!
//! ```text
//! defaults < files < remote < .env < APP_DB_* < bind_env < flags < overrides
//! ```
//!
//! Below the real environment, because a variable somebody exported for this
//! one run should beat a file checked into the repository.
//!
//! # It does not touch the process environment
//!
//! `dotenvy` and friends call `setenv`, which changes the environment of the
//! whole program to configure one struct. That is a side effect nobody asked
//! for, it is visible to every library in the process, and `setenv` is not
//! thread-safe — in Rust 2024 it is `unsafe` for that reason. This reads the
//! file and merges it, and nothing outside the configuration can tell.
//!
//! # What it parses
//!
//! `KEY=value`, one per line. `#` starts a comment, blank lines are skipped,
//! surrounding whitespace goes, and a value may be wrapped in single or double
//! quotes to keep whitespace or a `#`. `export KEY=value` is accepted, because
//! a `.env` is very often also sourced by a shell.
//!
//! Deliberately *not* supported: variable interpolation (`${OTHER}`) and
//! multi-line values. Both are shell features that every `.env` library
//! implements slightly differently, and a configuration file whose meaning
//! depends on which library read it is worse than one that refuses.

use std::collections::BTreeMap;
use std::path::Path;

use figment::value::{Dict, Value};
use figment::{Metadata, Profile, Provider};

use crate::error::{Error, ErrorKind, Origin};

/// How a value from a `.env` file names itself in a diagnostic.
pub(crate) const PREFIX: &str = "the file ";

/// Reads `path` into `KEY` → `value` pairs.
///
/// A file that is not there yields nothing, exactly as a missing config file
/// does: an optional `.env` next to a deployment that sets real variables is
/// the ordinary case.
///
/// # Errors
///
/// If the file exists but cannot be read, or holds a line that is not a
/// comment, blank, or `KEY=value`.
pub(crate) fn read(path: &Path) -> Result<BTreeMap<String, String>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(Error::new(ErrorKind::Io, error.to_string())
                .with_origin(Origin::File(path.to_owned())))
        }
    };

    parse(&text).map_err(|line| {
        Error::new(
            ErrorKind::Parse,
            format!("line {line} is not a comment, blank, or `KEY=value`"),
        )
        .with_origin(Origin::File(path.to_owned()))
    })
}

/// Parses the text, or reports the one-based line that could not be read.
fn parse(text: &str) -> Result<BTreeMap<String, String>, usize> {
    let mut entries = BTreeMap::new();

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // A `.env` is very often also sourced by a shell.
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();

        let Some((name, value)) = line.split_once('=') else {
            return Err(number + 1);
        };

        let name = name.trim();

        if name.is_empty() {
            return Err(number + 1);
        }

        entries.insert(name.to_owned(), unquote(value.trim()).to_owned());
    }

    Ok(entries)
}

/// Strips one matching pair of surrounding quotes.
///
/// Quotes are how a `.env` keeps trailing whitespace or a literal `#`. Only one
/// pair, and only a matching one: `"a'` is a value that begins with a quote.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }

    value
}

/// The variables from one `.env` file, as the environment layer sees them.
pub(crate) struct DotenvProvider {
    entries: BTreeMap<String, String>,
    named: String,
    prefix: String,
    key: String,
    nest: String,
    allow_empty: bool,
}

impl DotenvProvider {
    pub(crate) fn new(
        entries: BTreeMap<String, String>,
        path: &Path,
        prefix: &str,
        key: &str,
        nest: &str,
        allow_empty: bool,
    ) -> Self {
        Self {
            entries,
            named: format!("{PREFIX}{}", path.display()),
            prefix: prefix.to_owned(),
            key: key.to_owned(),
            nest: nest.to_owned(),
            allow_empty,
        }
    }
}

impl Provider for DotenvProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.named.clone())
    }

    fn data(&self) -> figment::Result<figment::value::Map<Profile, Dict>> {
        let mut values = Dict::new();

        for (name, text) in &self.entries {
            // Case-insensitively, because environment variables are written in
            // upper case and the prefix is given in whichever case suits.
            let Some(rest) = strip_prefix_ignoring_case(name, &self.prefix) else {
                continue;
            };

            // The same rule the real environment layer follows: `FOO=` — or
            // `FOO="  "`, whitespace being how a template renders "nothing"
            // just as often — is unset unless asked otherwise. One rule for
            // all three env-shaped layers; the loader documents it once.
            if text.trim().is_empty() && !self.allow_empty {
                continue;
            }

            let path = rest.to_ascii_lowercase().replace(&self.nest, ".");

            if path.is_empty() || path.split('.').any(str::is_empty) {
                continue;
            }

            let value = text
                .parse::<Value>()
                .unwrap_or_else(|_| Value::from(text.clone()));

            crate::layer::insert_path(&mut values, &path, value);
        }

        let mut map = figment::value::Map::new();
        map.insert(Profile::from(self.key.clone()), values);

        Ok(map)
    }
}

fn strip_prefix_ignoring_case<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    name.get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .and_then(|_| name.get(prefix.len()..))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let entries = parse("# a note\n\nAPP_HOST=localhost\n   \n").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries["APP_HOST"], "localhost");
    }

    #[test]
    fn export_is_accepted_because_a_shell_often_reads_the_same_file() {
        let entries = parse("export APP_HOST=localhost").unwrap();

        assert_eq!(entries["APP_HOST"], "localhost");
    }

    #[test]
    fn quotes_keep_what_trimming_would_take() {
        let entries = parse("A=\"  spaced  \"\nB='# not a comment'\nC=plain").unwrap();

        assert_eq!(entries["A"], "  spaced  ");
        assert_eq!(entries["B"], "# not a comment");
        assert_eq!(entries["C"], "plain");
    }

    #[test]
    fn a_mismatched_quote_is_part_of_the_value() {
        let entries = parse("A=\"unbalanced").unwrap();

        assert_eq!(entries["A"], "\"unbalanced");
    }

    #[test]
    fn a_value_may_contain_an_equals_sign() {
        let entries = parse("DSN=postgres://u:p@h/db?opt=1").unwrap();

        assert_eq!(entries["DSN"], "postgres://u:p@h/db?opt=1");
    }

    #[test]
    fn a_line_that_is_not_an_assignment_names_itself() {
        assert_eq!(parse("APP_HOST=ok\nnonsense\n").unwrap_err(), 2);
        assert_eq!(parse("=novalue").unwrap_err(), 1);
    }

    #[test]
    fn interpolation_is_left_alone_rather_than_half_implemented() {
        let entries = parse("A=${OTHER}").unwrap();

        assert_eq!(
            entries["A"], "${OTHER}",
            "a value whose meaning depends on which library read it is worse \
             than one that does nothing surprising"
        );
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert!(read(Path::new("/no/such/.env")).unwrap().is_empty());
    }
}
