//! The merge laws, as properties over generated layer stacks.
//!
//! The precedence order is the API (the Compatibility Contract's second
//! guarantee), and these are the algebraic halves of it:
//!
//! * **identity** — merging an empty document changes nothing, on either
//!   side: `merge(A, ∅) = merge(∅, A) = A`;
//! * **overlap-only override** — an overlay changes the result only at
//!   paths it supplies: where B has a leaf, the result is B's leaf; where
//!   nothing in B touches A's leaf or any of its ancestors, the result is
//!   A's leaf, untouched.
//!
//! The generator builds JSON documents (tables of tables of scalars), so
//! the parser is out of the frame — `flat_formats` and friends own that —
//! and what is under test is resolution itself.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use dynamic_config::{load, Format, LoadSpec, Source};

/// A generated document: at most three levels, keys from a small alphabet
/// so overlap between two documents actually happens.
#[derive(Arbitrary, Debug)]
struct Doc {
    entries: Vec<(Key, Node)>,
}

#[derive(Arbitrary, Debug)]
enum Node {
    Int(i32),
    Text(u8),
    Table(Vec<(Key, Leaf)>),
}

#[derive(Arbitrary, Debug)]
enum Leaf {
    Int(i32),
    Text(u8),
}

/// Seven keys, on purpose: a large alphabet would make two generated
/// documents disjoint almost always, and disjoint documents cannot
/// exercise override.
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

fn leaf_json(leaf: &Leaf) -> serde_json::Value {
    match leaf {
        Leaf::Int(n) => serde_json::json!(n),
        Leaf::Text(t) => serde_json::json!(format!("t{t}")),
    }
}

fn to_json(doc: &Doc) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for (key, node) in &doc.entries {
        let value = match node {
            Node::Int(n) => serde_json::json!(n),
            Node::Text(t) => serde_json::json!(format!("t{t}")),
            Node::Table(rows) => {
                let mut inner = serde_json::Map::new();

                for (k, leaf) in rows {
                    inner.insert(k.as_str().to_owned(), leaf_json(leaf));
                }

                serde_json::Value::Object(inner)
            }
        };

        // Later duplicates win, mirroring what any format's parser does.
        map.insert(key.as_str().to_owned(), value);
    }

    serde_json::Value::Object(map)
}

fn resolve(layers: &[String]) -> Option<serde_json::Value> {
    let sources: Vec<Source> = layers
        .iter()
        .map(|text| Source::inline(text, Format::Json))
        .collect();

    let spec = LoadSpec::new("doc", &sources).with_whole_document(true);

    load::<serde_json::Value>(&spec).ok()
}

/// Every leaf path of `value`, with its leaf.
fn leaves(value: &serde_json::Value, prefix: &str, out: &mut Vec<(String, serde_json::Value)>) {
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
        other => out.push((prefix.to_owned(), other.clone())),
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

    let Some(alone) = resolve(std::slice::from_ref(&a_text)) else {
        return;
    };

    // Identity, both sides.
    let empty = "{}".to_owned();

    let with_empty_above = resolve(&[a_text.clone(), empty.clone()]).expect("A loaded alone");
    let with_empty_below = resolve(&[empty, a_text.clone()]).expect("A loaded alone");

    assert_eq!(alone, with_empty_above, "merge(A, ∅) must be A");
    assert_eq!(alone, with_empty_below, "merge(∅, A) must be A");

    // Overlap-only override.
    let Some(merged) = resolve(&[a_text, b_text]) else {
        return;
    };

    let mut b_leaves = Vec::new();
    leaves(&b_json, "", &mut b_leaves);

    for (path, leaf) in &b_leaves {
        assert_eq!(
            lookup(&merged, path),
            Some(leaf),
            "where B supplies `{path}`, B wins"
        );
    }

    let mut a_leaves = Vec::new();
    leaves(&a_json, "", &mut a_leaves);

    for (path, leaf) in &a_leaves {
        // Untouched means: B holds nothing at the path, and no ancestor of
        // the path is a B-leaf (a leaf overwriting a table erases the
        // table's children, which IS an overlap).
        let touched = lookup(&b_json, path).is_some()
            || path
                .char_indices()
                .filter(|(_, c)| *c == '.')
                .map(|(i, _)| &path[..i])
                .any(|ancestor| {
                    lookup(&b_json, ancestor).is_some_and(|node| !node.is_object())
                });

        if !touched {
            assert_eq!(
                lookup(&merged, path),
                Some(leaf),
                "B never touched `{path}`; A's value must survive"
            );
        }
    }
});
