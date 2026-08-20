//! Every target type the backends claim, through this crate's reader.
//!
//! Both backends document the same list — primitives, sequences, maps,
//! options, every serde enum shape, user structs — because the list is
//! serde's rather than theirs. This crate deserializes with its own
//! reader, so "the same list" is a claim about *this* code, and the way
//! to hold it is to ask the original the same question and compare.
//!
//! Compared on the *outcome*, not the message: where a shape is refused
//! it must be refused by both, and where it reads it must read to the
//! same value.

#![cfg(all(feature = "json", feature = "figment"))]

use std::collections::{BTreeMap, HashMap, LinkedList, VecDeque};

use figment::providers::Format as _;
use serde::Deserialize;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Unit {
    Red,
    Blue,
}

#[derive(Debug, PartialEq, Deserialize)]
enum External {
    Timeout(u16),
    Window { from: u16, to: u16 },
    Pair(u8, u8),
    Nothing,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "kind")]
enum Internal {
    A { x: u8 },
    B { y: u8 },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "t", content = "c")]
enum Adjacent {
    A(u8),
    B(u8),
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
enum Untagged {
    N(u16),
    S(String),
}

#[derive(Debug, PartialEq, Deserialize)]
struct Newtype(u16);

#[derive(Debug, PartialEq, Deserialize)]
struct Tuple(u8, String);

#[derive(Debug, PartialEq, Deserialize)]
struct UnitStruct;

/// One document, one target type, both readings.
#[track_caller]
fn both<T>(what: &str, json: &str)
where
    T: serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
{
    let ours = dynamic_config::Value::parse(json, dynamic_config::Format::Json)
        .and_then(|value| value.get_as::<T>("a"))
        .map_err(|error| error.to_string());

    let theirs = figment::Figment::new()
        .merge(figment::providers::Json::string(json))
        .extract_inner::<T>("a")
        .map_err(|error| error.to_string());

    match (&ours, &theirs) {
        (Ok(ours), Ok(theirs)) => assert_eq!(ours, theirs, "{what}"),
        (Err(_), Err(_)) => {}
        _ => panic!("{what}: this crate says {ours:?}, the original {theirs:?}"),
    }
}

#[test]
fn every_type_the_backends_claim_reads_the_same() {
    both::<bool>("bool", r#"{"a": true}"#);
    both::<char>("char", r#"{"a": "x"}"#);
    both::<i8>("i8", r#"{"a": -8}"#);
    both::<i128>("i128", r#"{"a": -170141183460469231731687303715884105728}"#);
    both::<u128>(
        "u128 max",
        r#"{"a": 340282366920938463463374607431768211455}"#,
    );
    both::<usize>("usize", r#"{"a": 9}"#);
    both::<f32>("f32", r#"{"a": 1.5}"#);
    both::<Vec<u8>>("Vec<u8>", r#"{"a": [1,2,3]}"#);
    both::<[u8; 3]>("[u8; 3]", r#"{"a": [1,2,3]}"#);
    both::<LinkedList<u8>>("LinkedList", r#"{"a": [1,2]}"#);
    both::<VecDeque<u8>>("VecDeque", r#"{"a": [1,2]}"#);
    both::<HashMap<String, u8>>("HashMap", r#"{"a": {"k": 1}}"#);
    both::<BTreeMap<String, u8>>("BTreeMap", r#"{"a": {"k": 1}}"#);
    both::<Option<u16>>("Option some", r#"{"a": 5}"#);
    both::<Option<u16>>("Option null", r#"{"a": null}"#);
    both::<Unit>("unit enum", r#"{"a": "red"}"#);
    both::<External>("enum newtype variant", r#"{"a": {"Timeout": 30}}"#);
    both::<External>(
        "enum struct variant",
        r#"{"a": {"Window": {"from":1,"to":2}}}"#,
    );
    both::<External>("enum tuple variant", r#"{"a": {"Pair": [1,2]}}"#);
    both::<External>("enum unit variant", r#"{"a": "Nothing"}"#);
    both::<Internal>("internally tagged", r#"{"a": {"kind":"A","x":1}}"#);
    both::<Adjacent>("adjacently tagged", r#"{"a": {"t":"A","c":1}}"#);
    both::<Untagged>("untagged (number)", r#"{"a": 7}"#);
    both::<Untagged>("untagged (string)", r#"{"a": "seven"}"#);
    both::<Newtype>("newtype struct", r#"{"a": 5}"#);
    both::<Tuple>("tuple struct", r#"{"a": [1, "x"]}"#);
    both::<UnitStruct>("unit struct", r#"{"a": null}"#);
    both::<(u8, u8)>("tuple", r#"{"a": [1,2]}"#);
    both::<std::net::IpAddr>("IpAddr", r#"{"a": "127.0.0.1"}"#);
    both::<std::path::PathBuf>("PathBuf", r#"{"a": "/etc"}"#);
    both::<String>("loose: number as String", r#"{"a": 42}"#);
    both::<u16>("loose: string as u16", r#"{"a": "8080"}"#);
    both::<bool>("loose: string as bool", r#"{"a": "true"}"#);
}
