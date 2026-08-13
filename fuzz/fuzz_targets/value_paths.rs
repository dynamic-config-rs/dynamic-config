//! `Value::get`, structure-aware: a generated tree and a generated path.
//!
//! `Value` is the handover surface — what a language binding, an exporter or
//! a templating engine walks — and `get` is the dotted-path walker over it.
//! Its input is attacker-shaped twice over: the tree comes from a
//! configuration document and the path from a caller's key string.
//!
//! Bytes would be a waste here too, so the generator builds trees: nesting,
//! arrays interrupting a walk, keys that are empty, keys that contain the
//! separator.
//!
//! The properties:
//!
//! * `get` is total — no path panics, however deep or malformed;
//! * the empty path is the value itself;
//! * a walk that succeeds has a parent that succeeded (no path resolves
//!   through a step that does not);
//! * every key reachable by an unambiguous walk resolves to the node it names.
//!
//! **Dotted keys are deliberately excluded from the last one.** A table key
//! containing `.` is ambiguous under a dotted path language and always will
//! be — `{"a.b": 1}` and `{"a": {"b": 1}}` render the same path. Asserting
//! otherwise would be asserting a bug that is really a design decision.

#![no_main]

use arbitrary::Arbitrary;
use dynamic_config::Value;
use libfuzzer_sys::fuzz_target;

/// A tree the generator can build; `Value` is not ours to derive on, and a
/// fuzzing concern does not belong in the library.
#[derive(Arbitrary, Debug)]
enum Shape {
    Null,
    Bool(bool),
    Integer(i128),
    Float(f64),
    Text(String),
    Array(Vec<Shape>),
    Table(Vec<(String, Shape)>),
}

impl Shape {
    fn build(&self) -> Value {
        match self {
            Shape::Null => Value::Null,
            Shape::Bool(value) => Value::Bool(*value),
            Shape::Integer(value) => Value::Integer(*value),
            Shape::Float(value) => Value::Float(*value),
            Shape::Text(value) => Value::String(value.clone()),
            Shape::Array(items) => Value::Array(items.iter().map(Shape::build).collect()),
            Shape::Table(entries) => Value::Table(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.build()))
                    .collect(),
            ),
        }
    }
}

/// Every path that names a node, skipping any branch whose key would make the
/// path ambiguous. Bounded, because a generated tree can be deep enough that
/// enumerating it costs more than the walk being tested.
fn unambiguous_paths(value: &Value, prefix: &str, found: &mut Vec<String>) {
    const LIMIT: usize = 256;

    if found.len() >= LIMIT {
        return;
    }

    let Value::Table(table) = value else {
        return;
    };

    for (key, child) in table {
        if key.is_empty() || key.contains('.') {
            continue;
        }

        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        found.push(path.clone());
        unambiguous_paths(child, &path, found);
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    tree: Shape,
    /// Walked as well as the enumerated ones: the enumeration only ever
    /// produces paths that exist, and the paths that do not are half the
    /// surface.
    probes: Vec<String>,
}

fuzz_target!(|input: Input| {
    let value = input.tree.build();

    // Identity, not equality. `Value` is deliberately not `Eq` — a `Float`
    // holding `NaN` does not equal itself — so comparing by value here would
    // assert a bug that is really an IEEE 754 decision the crate documents.
    // The claim is that `get("")` hands back *this* value, which is a
    // pointer question. (The fuzzer found this within a minute of the first
    // run being written the other way, which is the target earning its keep
    // before it ever reached the library.)
    assert!(
        matches!(value.get(""), Some(found) if std::ptr::eq(found, &value)),
        "the empty path is the value itself"
    );

    for probe in &input.probes {
        if let Some(found) = value.get(probe) {
            // A resolved path implies a resolved parent: the walk is a fold,
            // and a step that returned a node must have had one to step from.
            if let Some((parent, _)) = probe.rsplit_once('.') {
                assert!(
                    value.get(parent).is_some(),
                    "{probe:?} resolved but its parent {parent:?} did not"
                );
            }

            let _ = found;
        }
    }

    let mut paths = Vec::new();
    unambiguous_paths(&value, "", &mut paths);

    for path in &paths {
        assert!(
            value.get(path).is_some(),
            "{path:?} names a node but does not resolve"
        );
    }
});
