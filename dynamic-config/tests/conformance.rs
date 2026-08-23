//! The conformance suite's Rust runner — see `conformance/README.md`.
//!
//! One `#[test]`, every case, sequentially: cases set environment
//! variables, and the environment is process-global. Each case uses a
//! prefix unique to itself, and this runner still removes what it set,
//! so an aborted case cannot leak into the next.
//!
//! The Python and Node repositories carry runners of the same size over
//! the same directory; a disagreement anywhere names the case.

#![cfg(all(feature = "toml", feature = "json", feature = "dotenv"))]

use std::collections::BTreeMap;
use std::path::Path;

use dynamic_config::{load, Layer, LoadSpec, Source};

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Args {
    key: String,
    #[serde(default)]
    env_prefix: Option<String>,
    #[serde(default)]
    profile_env: Option<String>,
    #[serde(default)]
    defaults: Option<serde_json::Value>,
    #[serde(default)]
    set: Option<serde_json::Value>,
    #[serde(default)]
    overrides: Option<serde_json::Value>,
    #[serde(default)]
    secrets_dir: Option<String>,
    #[serde(default)]
    env_files: Vec<String>,
    #[serde(default)]
    aliases: Option<BTreeMap<String, String>>,
    #[serde(default)]
    whole_document: bool,
    #[serde(default)]
    extra_missing_file: Option<String>,
    /// The dotted paths a fingerprinting case declares secret.
    #[serde(default)]
    secrets: Vec<String>,
    /// Present only in a case that pins a digest. It is pinned here rather
    /// than in a suite of its own because a fingerprint is a property *of*
    /// the resolution: the three languages have to agree on the digest of
    /// the tree they already agree on.
    #[serde(default)]
    expected_fingerprint: Option<String>,
}

fn layer_of(value: &serde_json::Value) -> Layer {
    let layer = Layer::new();

    fn walk(layer: &Layer, prefix: &str, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) if !map.is_empty() => {
                for (key, inner) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };

                    walk(layer, &path, inner);
                }
            }
            // The empty-object root: an empty layer is the identity, which
            // is exactly what the empty-layer-identity case asserts.
            serde_json::Value::Object(_) => {}
            other => layer.set(prefix, other).expect("a layer accepts JSON"),
        }
    }

    walk(&layer, "", value);

    layer
}

#[test]
fn every_case_resolves_to_its_expectation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../conformance/cases");
    let mut cases: Vec<_> = std::fs::read_dir(&root)
        .expect("the conformance cases directory exists")
        .map(|entry| entry.expect("a readable entry").path())
        .filter(|path| path.is_dir())
        .collect();

    cases.sort();
    assert!(!cases.is_empty(), "no cases found under {}", root.display());

    let mut failures = Vec::new();

    for case in &cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();

        if let Err(reason) = run_case(case) {
            failures.push(format!("{name}: {reason}"));
        }
    }

    assert!(
        failures.is_empty(),
        "conformance disagreements:\n  {}",
        failures.join("\n  ")
    );
}

fn run_case(case: &Path) -> Result<(), String> {
    let read = |file: &str| -> Result<String, String> {
        std::fs::read_to_string(case.join(file)).map_err(|error| format!("{file}: {error}"))
    };

    let args: Args =
        serde_json::from_str(&read("args.json")?).map_err(|error| format!("args.json: {error}"))?;
    let env: BTreeMap<String, String> =
        serde_json::from_str(&read("env.json")?).map_err(|error| format!("env.json: {error}"))?;
    let expected: serde_json::Value = serde_json::from_str(&read("expected.json")?)
        .map_err(|error| format!("expected.json: {error}"))?;

    for (key, value) in &env {
        // Process-global on purpose: the layer under test reads the real
        // environment, each case's prefix is its own, and the cases run
        // one after another inside a single #[test].
        std::env::set_var(key, value);
    }

    let outcome = resolve(case, &args);

    for key in env.keys() {
        std::env::remove_var(key);
    }

    let (resolved, fingerprint) = outcome?;

    if resolved != expected {
        return Err(format!(
            "resolved {} but expected {}",
            serde_json::to_string(&resolved).unwrap(),
            serde_json::to_string(&expected).unwrap(),
        ));
    }

    match (&args.expected_fingerprint, &fingerprint) {
        (Some(expected), Some(actual)) if actual != expected => {
            Err(format!("fingerprinted {actual} but expected {expected}"))
        }
        _ => Ok(()),
    }
}

/// The resolved section, and the digest of it when the case pins one.
fn resolve(case: &Path, args: &Args) -> Result<(serde_json::Value, Option<String>), String> {
    let config = case.join("config.toml");
    let config = config.to_str().ok_or("a UTF-8 path")?;

    let missing = args
        .extra_missing_file
        .as_ref()
        .map(|name| case.join(name).to_string_lossy().into_owned());

    let mut sources = vec![Source::file(config, dynamic_config::Format::Toml)];

    if let Some(missing) = &missing {
        // Listed files are optional by contract: a missing one is skipped
        // like any other absent layer, which is what the case asserts.
        sources.push(Source::file(missing, dynamic_config::Format::Toml));
    }

    let env_files: Vec<String> = args
        .env_files
        .iter()
        .map(|name| case.join(name).to_string_lossy().into_owned())
        .collect();
    let env_file_refs: Vec<&str> = env_files.iter().map(String::as_str).collect();

    let secrets = args
        .secrets_dir
        .as_ref()
        .map(|dir| case.join(dir).to_string_lossy().into_owned());

    let defaults = args.defaults.as_ref().map(layer_of);
    let flags = args.set.as_ref().map(layer_of);
    let overrides = args.overrides.as_ref().map(layer_of);

    let aliases = args.aliases.as_ref().map(|pairs| {
        let aliases = dynamic_config::Aliases::new();

        for (alias, canonical) in pairs {
            aliases
                .add(alias, canonical)
                .expect("a conformance alias is well-formed");
        }

        aliases
    });

    let mut spec = LoadSpec::new(&args.key, &sources);

    if let Some(prefix) = &args.env_prefix {
        spec = spec.with_env(prefix);
    }

    if let Some(variable) = &args.profile_env {
        spec = spec.with_profile_env(variable);
    }

    if let Some(defaults) = &defaults {
        spec = spec.with_defaults(defaults);
    }

    if let Some(flags) = &flags {
        spec = spec.with_flags(flags);
    }

    if let Some(overrides) = &overrides {
        spec = spec.with_overrides(overrides);
    }

    if let Some(secrets) = &secrets {
        spec = spec.with_secrets_dir(secrets);
    }

    if !env_file_refs.is_empty() {
        spec = spec.with_env_files(&env_file_refs);
    }

    if let Some(aliases) = &aliases {
        spec = spec.with_aliases(aliases);
    }

    spec = spec.with_whole_document(args.whole_document);

    let resolved =
        load::<serde_json::Value>(&spec).map_err(|error| format!("load refused: {error}"))?;

    // Through the public `snapshot` door rather than a builder, so the
    // digest is over the same tree the comparison above just made — and so
    // this runner stays the ~50 lines of glue the other two are.
    let fingerprint = if args.expected_fingerprint.is_some() {
        let secrets: Vec<&str> = args.secrets.iter().map(String::as_str).collect();

        Some(
            dynamic_config::snapshot(&spec)
                .map_err(|error| format!("snapshot refused: {error}"))?
                .fingerprint(&secrets),
        )
    } else {
        None
    };

    Ok((resolved, fingerprint))
}
