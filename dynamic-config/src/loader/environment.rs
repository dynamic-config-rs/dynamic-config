//! The environment layers: prefixed variables, and `.env` files below them.

use crate::error::{Error, ErrorKind};
use crate::source::LoadSpec;

/// Each `.env` file's contribution, in the order they were listed.
#[cfg(feature = "dotenv")]
pub(super) fn collect_env_files(
    into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    let Some(prefix) = spec.full_env_prefix() else {
        return Ok(());
    };

    for file in spec.env_files {
        let path = std::path::Path::new(file);
        let entries = crate::dotenv::read(path)?;

        if entries.is_empty() {
            continue;
        }

        if spec.strict_env {
            if let Some((name, _)) = entries.iter().find(|(_, value)| is_ambiguous(value)) {
                return Err(ambiguous(name).with_origin(crate::Origin::File(path.to_owned())));
            }
        }

        let values = crate::dotenv::tree(&entries, &prefix, spec.nest, spec.allow_empty_env);

        if !values.is_empty() {
            into.layer(".env", crate::Origin::File(path.to_owned()), values);
        }
    }

    Ok(())
}

/// With the feature off there are no `.env` files to read.
#[cfg(not(feature = "dotenv"))]
pub(super) fn collect_env_files(
    _into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    if spec.env_files.is_empty() {
        return Ok(());
    }

    Err(Error::new(
        ErrorKind::Backend,
        "`.env` files need the `dotenv` feature",
    ))
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
    // The same walk the layer itself does, so a variable one of them sees is
    // a variable the other sees — a strict-mode check that reads a different
    // set from the layer it guards is a check that can be walked around.
    for (segments, value) in crate::env_layer::variables(prefix, "__") {
        if is_ambiguous(&value) {
            return Err(ambiguous(&format!(
                "{prefix}{}",
                segments.join("__").to_ascii_uppercase()
            )));
        }
    }

    Ok(())
}
