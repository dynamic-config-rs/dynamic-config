//! Provenance names the winner: `source_of(path)` must point at the layer
//! that actually supplied the resolved value — the Compatibility Contract's
//! fourth guarantee, as a property over generated two-layer stacks.
//!
//! The generator reuses `merge_laws`' vocabulary: small documents over a
//! seven-key alphabet, so the two layers overlap often enough to make the
//! question interesting. For every leaf of the *resolved* document, the
//! answer must be: the second layer when it supplied the leaf, the first
//! when only it did — and never absent for a path that resolved.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use dynamic_config::{load, source_of, Format, LoadSpec, Source};

#[derive(Arbitrary, Debug)]
struct Doc {
    entries: Vec<(Key, Node)>,
}

#[derive(Arbitrary, Debug)]
enum Node {
    Int(i32),
    Table(Vec<(Key, i32)>),
}

#[derive(Arbitrary, Debug, Clone, Copy, PartialEq)]
enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
}

impl Key {
    fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
            Self::D => "d",
            Self::E => "e",
            Self::F => "f",
            Self::G => "g",
        }
    }
}

fn to_json(doc: &Doc) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for (key, node) in &doc.entries {
        let value = match node {
            Node::Int(n) => serde_json::json!(n),
            Node::Table(rows) => {
                let mut inner = serde_json::Map::new();

                for (k, n) in rows {
                    inner.insert(k.as_str().to_owned(), serde_json::json!(n));
                }

                serde_json::Value::Object(inner)
            }
        };

        map.insert(key.as_str().to_owned(), value);
    }

    serde_json::Value::Object(map)
}

fn leaves(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };

                leaves(v, &path, out);
            }
        }
        _ => out.push(prefix.to_owned()),
    }
}

fn lookup<'v>(value: &'v serde_json::Value, path: &str) -> Option<&'v serde_json::Value> {
    let mut node = value;

    for step in path.split('.') {
        node = node.as_object()?.get(step)?;
    }

    Some(node)
}

fuzz_target!(|input: (Doc, Doc)| {
    let (a, b) = input;
    let a_json = to_json(&a);
    let b_json = to_json(&b);
    let a_text = a_json.to_string();
    let b_text = b_json.to_string();

    let sources = [
        Source::inline(&a_text, Format::Json),
        Source::inline(&b_text, Format::Json),
    ];
    let spec = LoadSpec::new("doc", &sources).with_whole_document(true);

    let Ok(resolved) = load::<serde_json::Value>(&spec) else {
        return;
    };

    let mut resolved_leaves = Vec::new();
    leaves(&resolved, "", &mut resolved_leaves);

    for path in &resolved_leaves {
        let origin = source_of(&spec, path).expect("a resolved load explains itself");

        let origin = origin.unwrap_or_else(|| {
            panic!("`{path}` resolved but has no origin");
        });

        let described = format!("{origin:?}");

        // Which layer actually supplied the winning leaf?
        let expected_second = lookup(&b_json, path) == lookup(&resolved, path)
            && lookup(&b_json, path).is_some();

        // The two inline layers render distinguishably in an `Origin` by
        // their index; asserting on the rendered form keeps this decoupled
        // from `Origin`'s exact shape while still failing when the wrong
        // layer is named.
        if expected_second {
            assert!(
                !described.contains("inline #1") || lookup(&a_json, path) == lookup(&b_json, path),
                "`{path}` was supplied by the second layer; origin says {described}"
            );
        } else {
            assert!(
                !described.contains("inline #2"),
                "`{path}` was supplied by the first layer; origin says {described}"
            );
        }
    }
});
