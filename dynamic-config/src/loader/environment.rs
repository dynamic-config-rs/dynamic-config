//! The environment layers: prefixed variables, and `.env` files below them.

use figment::providers::Env;
use figment::Figment;
#[cfg(feature = "dotenv")]
use std::path::Path;

use crate::error::{Error, ErrorKind};
use crate::source::LoadSpec;

/// The environment provider for one section.
///
/// figment has no value-level filter, so dropping empty variables means finding
/// them first and filtering by name. The filter is installed *before* `split`,
/// so it sees the prefix-stripped name with its `__` separators intact — the
/// same shape `empty_keys` produces.
pub(super) fn environment(prefix: &str, key: &str, nest: &str, allow_empty: bool) -> Env {
    let mut env = Env::prefixed(prefix);

    if !allow_empty {
        let empty = empty_keys(prefix);

        if !empty.is_empty() {
            env = env.filter_map(move |name| {
                let is_empty = empty
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(name.as_str()));

                (!is_empty).then(|| name.into())
            });
        }
    }

    env.split(nest).profile(key)
}

/// The spellings strict mode refuses: they read like booleans (or like
/// nothing) and arrive as strings, which is silently right in a `String`
/// field and silently wrong everywhere else.
const AMBIGUOUS: &[&str] = &["yes", "no", "on", "off", "null", "nil", "none"];

fn is_ambiguous(value: &str) -> bool {
    let trimmed = value.trim();

    AMBIGUOUS
        .iter()
        .any(|candidate| trimmed.eq_ignore_ascii_case(candidate))
}

/// One refusal, naming the variable and what to write instead — and not the
/// value: the ambiguous family is seven known words, but a diagnostic that
/// echoes environment values is a diagnostic one copy-paste away from
/// echoing a secret. Values stay out of messages everywhere, including here.
fn ambiguous(variable: &str) -> Error {
    Error::new(
        ErrorKind::Env,
        format!(
            "`{variable}` is set to one of the ambiguous yes/no/on/off \
             spellings, which `strict_env` refuses: it would arrive as a \
             string, not a boolean; write `true`, `false`, or the value you \
             mean"
        ),
    )
}

/// Rejects ambiguous values among the real environment variables under
/// `prefix`. Strict mode's whole job; a no-op without it.
pub(super) fn reject_ambiguous(prefix: &str) -> Result<(), Error> {
    // `vars_os` for the same reason as `empty_keys` below: a non-UTF-8 name
    // cannot match an ASCII prefix, and this must not panic on one.
    for (name, value) in std::env::vars_os() {
        let Ok(name) = name.into_string() else {
            continue;
        };
        let Ok(value) = value.into_string() else {
            continue;
        };

        let matches_prefix = name
            .get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix));

        if matches_prefix && is_ambiguous(&value) {
            return Err(ambiguous(&name));
        }
    }

    Ok(())
}

/// Prefix-stripped names of the variables under `prefix` whose value is blank.
fn empty_keys(prefix: &str) -> Vec<String> {
    // `vars_os`, never `vars`: `vars()` panics on the first non-UTF-8 name or
    // value in the environment, and this runs on *every* load — a stray byte
    // in some inherited locale variable must not turn every reload into a
    // panic. A name that is not UTF-8 cannot match an ASCII prefix; a value
    // that is not UTF-8 is not blank. Both are skipped, not fatal.
    std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let value = value.into_string().unwrap_or_else(|_| "x".to_owned());

            Some((name, value))
        })
        .filter(|(_, value)| value.trim().is_empty())
        .filter_map(|(name, _)| {
            name.get(..prefix.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
                .map(|_| name[prefix.len()..].to_owned())
        })
        .collect()
}

/// Merges every `.env` file, in order, below the real environment.
///
/// Nothing at all without an `env` prefix: a `.env` holds variable names, and
/// without a prefix there is no rule for which of them belong to this section.
#[cfg(feature = "dotenv")]
pub(super) fn merge_env_files(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let Some(prefix) = spec.full_env_prefix() else {
        return Ok(figment);
    };

    for file in spec.env_files {
        let path = Path::new(file);
        let entries = crate::dotenv::read(path)?;

        if entries.is_empty() {
            continue;
        }

        if spec.strict_env {
            if let Some((name, _)) = entries.iter().find(|(_, value)| is_ambiguous(value)) {
                return Err(ambiguous(name).with_origin(crate::Origin::File(path.to_owned())));
            }
        }

        figment = figment.merge(crate::dotenv::DotenvProvider::new(
            entries,
            path,
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ));
    }

    Ok(figment)
}

/// Reachable only through the macro-free API: `#[dynamic_config]` turns
/// `env_files` without the feature into a compile error naming it.
#[cfg(not(feature = "dotenv"))]
pub(super) fn merge_env_files(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if spec.env_files.is_empty() {
        return Ok(figment);
    }

    Err(Error::new(
        ErrorKind::Backend,
        "`.env` files need the `dotenv` feature; add features = [\"dotenv\"] \
         to your dynamic-config dependency",
    ))
}
