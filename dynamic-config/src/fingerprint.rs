//! A stable digest of a resolved configuration.
//!
//! The question it answers is "are these two processes running the same
//! configuration?", asked across a fleet where diffing two documents is not
//! available — one pod prints a string, another prints a string, and the
//! strings either match or they do not.
//!
//! # Why it is safe to log
//!
//! Secrets do not reach the hash as values. Every declared secret is
//! replaced by a fixed marker before hashing, so a fingerprint moves when a
//! secret *appears or disappears* — a real change in the shape of a
//! configuration — and stays put when one merely *rotates*. That second
//! property is the point: a fingerprint is meant for a metric label, a log
//! line and a `kubectl describe`, and a value-sensitive digest of a
//! password is an oracle for it.
//!
//! # Why the encoding is written out
//!
//! The bytes fed to the hash are produced here rather than borrowed from a
//! serializer, because the digest is an API. It has to survive a
//! `serde_json` upgrade, a toolchain upgrade, and the same document
//! arriving as TOML on one host and YAML on another. So the tree is walked
//! in its own order — [`Value`] tables are `BTreeMap`s, which sort — and
//! each node is written with a tag byte and a length, so no two shapes can
//! encode to the same bytes.
//!
//! The predecessor hashed `Debug` output through `DefaultHasher`, which is
//! `SipHash` with an unspecified-across-releases implementation: a Rust
//! upgrade could silently change every fingerprint on disk.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::value::Value;

/// The shape a resolved section has: keys at the top, values under them.
type Document = BTreeMap<String, Value>;

/// The prefix of the key a masked secret leaves behind.
///
/// Masking works by *position*: the value is emptied and a flag key is put
/// beside it, named with a prefix no configuration key can be. Doing it
/// this way rather than by substituting a magic value means an ordinary
/// string that happens to equal that value cannot fingerprint as though it
/// were a secret.
const MASKED: &str = "\u{0}dynamic-config:secret\u{0}";

/// The digest of `values`, with the paths in `secrets` masked.
///
/// Returns the hex digest without a prefix; callers that render it for
/// humans add `sha256:`.
pub(crate) fn of(values: &Document, secrets: &[&str]) -> String {
    let mut masked = values.clone();

    for secret in secrets {
        mask_path(&mut masked, secret);
    }

    let mut hasher = Sha256::new();
    write_table(&mut hasher, &masked);

    let digest = hasher.finalize();
    let mut rendered = String::with_capacity(digest.len() * 2);

    for byte in digest {
        use std::fmt::Write;

        // `write!` to a `String` cannot fail; the result is discarded
        // rather than unwrapped so this stays panic-free by construction.
        let _ = write!(rendered, "{byte:02x}");
    }

    rendered
}

/// [`of`], rendered the way it is meant to be read and logged.
pub(crate) fn rendered(values: &Document, secrets: &[&str]) -> String {
    format!("sha256:{}", of(values, secrets))
}

/// Replaces the value at a dotted `path` with the marker, if the path leads
/// anywhere.
///
/// Replacing rather than removing, so that a configuration which grows a
/// secret is not confused with one that never had the key.
fn mask_path(table: &mut Document, path: &str) {
    match path.split_once('.') {
        None => {
            if let Some(slot) = table.get_mut(path) {
                // Emptied, not removed: a configuration that *grows* a
                // secret must not fingerprint as one that never had the key.
                *slot = Value::Null;
                table.insert(format!("{MASKED}{path}"), Value::Bool(true));
            }
        }
        Some((head, rest)) => {
            if let Some(Value::Table(nested)) = table.get_mut(head) {
                mask_path(nested, rest);
            }
        }
    }
}

fn write_table(hasher: &mut Sha256, table: &Document) {
    hasher.update(*b"T");
    hasher.update(u64::try_from(table.len()).unwrap_or(u64::MAX).to_be_bytes());

    // `BTreeMap`, so this is sorted, so two runs agree — and so do two
    // documents that were written with their keys in different orders.
    for (key, value) in table {
        write_bytes(hasher, key.as_bytes());
        write_value(hasher, value);
    }
}

fn write_value(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Table(table) => write_table(hasher, table),
        Value::Array(values) => {
            hasher.update(*b"A");
            hasher.update(
                u64::try_from(values.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );

            for value in values {
                write_value(hasher, value);
            }
        }
        Value::String(text) => {
            hasher.update(*b"S");
            write_bytes(hasher, text.as_bytes());
        }
        Value::Integer(number) => {
            hasher.update(*b"I");
            hasher.update(number.to_be_bytes());
        }
        Value::Float(number) => {
            hasher.update(*b"F");

            // Through the bits rather than a rendering: `1.0` and `1` are
            // different values here, and a formatter's idea of how many
            // digits to print is not something a digest should inherit.
            hasher.update(number.to_bits().to_be_bytes());
        }
        Value::Bool(flag) => {
            hasher.update([b'B', u8::from(*flag)]);
        }
        Value::Null => {
            hasher.update(*b"N");
        }
    }
}

/// Length-prefixed, so `{"ab": …}` and `{"a": {"b": …}}` cannot collide.
fn write_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pairs: &[(&str, Value)]) -> Document {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn the_same_document_hashes_the_same() {
        let one = table(&[("host", Value::String("db".to_owned()))]);
        let two = table(&[("host", Value::String("db".to_owned()))]);

        assert_eq!(of(&one, &[]), of(&two, &[]));
    }

    #[test]
    fn a_changed_value_moves_it() {
        let before = table(&[("port", Value::Integer(80))]);
        let after = table(&[("port", Value::Integer(443))]);

        assert_ne!(of(&before, &[]), of(&after, &[]));
    }

    #[test]
    fn a_rotated_secret_does_not_move_it() {
        let before = table(&[
            ("host", Value::String("db".to_owned())),
            ("password", Value::String("first".to_owned())),
        ]);
        let after = table(&[
            ("host", Value::String("db".to_owned())),
            ("password", Value::String("second".to_owned())),
        ]);

        assert_eq!(
            of(&before, &["password"]),
            of(&after, &["password"]),
            "a fingerprint that moved when a password rotated would be an \
             oracle for the password"
        );
    }

    #[test]
    fn a_secret_appearing_does_move_it() {
        let without = table(&[("host", Value::String("db".to_owned()))]);
        let with = table(&[
            ("host", Value::String("db".to_owned())),
            ("password", Value::String("first".to_owned())),
        ]);

        assert_ne!(
            of(&without, &["password"]),
            of(&with, &["password"]),
            "gaining a secret is a change in shape, not a change in value"
        );
    }

    #[test]
    fn nesting_cannot_be_flattened_into_a_collision() {
        let nested = table(&[("a", Value::Table(table(&[("b", Value::Integer(1))])))]);
        let flat = table(&[("a.b", Value::Integer(1))]);

        assert_ne!(of(&nested, &[]), of(&flat, &[]));
    }

    #[test]
    fn keys_of_different_lengths_cannot_collide() {
        let one = table(&[("ab", Value::String("c".to_owned()))]);
        let two = table(&[("a", Value::String("bc".to_owned()))]);

        assert_ne!(of(&one, &[]), of(&two, &[]));
    }

    #[test]
    fn a_float_and_an_integer_are_different_values() {
        let integer = table(&[("n", Value::Integer(1))]);
        let float = table(&[("n", Value::Float(1.0))]);

        assert_ne!(of(&integer, &[]), of(&float, &[]));
    }

    #[test]
    fn masking_is_by_position_and_not_by_content() {
        // A document whose plain value happens to look like the marker must
        // not fingerprint as though it held a secret there.
        let impostor = table(&[("password", Value::String(MASKED.to_owned()))]);
        let genuine = table(&[("password", Value::String("hunter2".to_owned()))]);

        assert_ne!(of(&impostor, &[]), of(&genuine, &["password"]));
    }

    #[test]
    fn an_absent_secret_and_a_null_one_are_different_shapes() {
        let absent = table(&[("host", Value::String("db".to_owned()))]);
        let null = table(&[
            ("host", Value::String("db".to_owned())),
            ("password", Value::Null),
        ]);

        assert_ne!(of(&absent, &["password"]), of(&null, &["password"]));
    }
}
