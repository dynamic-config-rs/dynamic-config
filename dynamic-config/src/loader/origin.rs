//! Error translation and origin tracing: turning `figment::Error` into this
//! crate's [`Error`] with the value redacted, and naming the layer a value
//! came from.

use crate::error::Origin;

/// Drops the backtick-quoted payload serde itself embeds in a message.
///
/// The figment kinds above are scrubbed structurally, but a serde error
/// that reaches figment as plain text — `serde_json` failing inside a
/// whole-document extract renders `invalid type: integer \`555511…\`,
/// expected a map` — carries the offending *value* in backticks, and a
/// password mistyped into a numeric field is exactly that value. The
/// found-value span goes; the expected type, which serde writes in plain
/// words after the comma, survives. Messages outside serde's two
/// value-carrying prefixes pass through untouched, so `unknown field
/// \`retries\`` keeps naming the field — a field name is a key path,
/// which every diagnostic here is allowed to say.
///
/// Found by the `lkg_serves_previous` fuzz target, first corpus run.
pub(crate) fn without_backticked_values(message: &str) -> String {
    let leaky = message.contains("invalid type:") || message.contains("invalid value:");

    if !leaky {
        return message.to_owned();
    }

    // serde spells the found value two ways: backticks for numbers and
    // friends, double quotes for strings — the second being the one a
    // pasted password actually arrives in.
    let mut out = String::with_capacity(message.len());
    let mut rest = message;

    loop {
        let tick = rest.find('`');
        let quote = rest.find('"');

        let (open, mark) = match (tick, quote) {
            (Some(t), Some(q)) if t < q => (t, '`'),
            (Some(t), None) => (t, '`'),
            (_, Some(q)) => (q, '"'),
            (None, None) => break,
        };

        let Some(close) = rest[open + 1..].find(mark) else {
            break;
        };

        out.push_str(&rest[..open]);
        out.push(mark);
        out.push_str("<redacted>");
        out.push(mark);
        rest = &rest[open + 1 + close + 1..];
    }

    out.push_str(rest);
    out
}

/// Upgrades a prefix-grained environment origin to the exact variable.
///
/// figment attaches metadata per provider and the prefixed environment is
/// one provider, so the trail ends at `APP_DB_*`. But the crate holds
/// every ingredient the full name is made of — the prefix (in the origin
/// itself), the key path the question is about, and the nesting separator
/// — so the variable is *derived*: path segments uppercased and joined by
/// the separator, appended to the prefix. A naming convention rather than
/// a measurement, which is why `tests/loader.rs` pins it: if figment ever
/// changes the convention, the drift shows up there and not in a bug
/// report.
///
/// Derived, then *checked*: the composed name is only claimed when that
/// variable actually exists in the environment. An aliased value carries
/// the destination path while the variable that supplied it spells the
/// old one — deriving from the path would name a variable nobody set —
/// and the honest fallback for any composition the environment does not
/// confirm is the prefix wildcard the trail already ended at.
pub(super) fn refine_env<'a>(
    origin: Origin,
    path: impl Iterator<Item = &'a str>,
    nest: &str,
) -> Origin {
    let Origin::Env(prefix) = &origin else {
        return origin;
    };
    let Some(stem) = prefix.strip_suffix('*') else {
        return origin;
    };

    let segments: Vec<String> = path.map(str::to_ascii_uppercase).collect();

    if segments.is_empty() {
        return origin;
    }

    let variable = format!("{stem}{}", segments.join(&nest.to_ascii_uppercase()));

    if std::env::var_os(&variable).is_none() {
        return origin;
    }

    Origin::Env(variable)
}
