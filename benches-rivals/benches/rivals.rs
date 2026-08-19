//! The same work, three crates: load a 100-key layered document into a
//! typed value. Architectural cost made visible, not a race — the book's
//! positioning page says why cross-crate numbers are read that way, and
//! the cross-LANGUAGE positioning (Viper, Koanf, Dynaconf,
//! pydantic-settings) lives on that page as hand-run numbers, not here.
//!
//! ```text
//! cd benches-rivals && cargo bench
//! ```

use criterion::{criterion_group, criterion_main, Criterion};

fn document(keys: usize) -> String {
    let mut body = String::from("[app]\n");

    for k in 0..keys {
        body.push_str(&format!("k{k} = {k}\n"));
    }

    body
}

fn write_fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("dynamic-config-rivals");
    std::fs::create_dir_all(&dir).expect("scratch");
    let path = dir.join("config.toml");
    std::fs::write(&path, document(100)).expect("scratch");
    path
}

fn load_compare(criterion: &mut Criterion) {
    let path = write_fixture();
    let path_str = path.to_str().expect("utf-8");

    let mut group = criterion.benchmark_group("load_100_keys");

    group.bench_function("dynamic-config", |bench| {
        bench.iter(|| {
            let sources = [dynamic_config::Source::file(
                path_str,
                dynamic_config::Format::Toml,
            )];
            let spec = dynamic_config::LoadSpec::new("app", &sources);

            dynamic_config::load::<serde_json::Value>(&spec).expect("loads")
        });
    });

    group.bench_function("config-rs", |bench| {
        bench.iter(|| {
            config::Config::builder()
                .add_source(config::File::with_name(path_str))
                .build()
                .expect("loads")
                .get::<serde_json::Value>("app")
                .expect("the section")
        });
    });

    group.bench_function("figment", |bench| {
        bench.iter(|| {
            use figment::providers::Format as _;

            figment::Figment::new()
                .merge(figment::providers::Toml::file(path_str))
                .extract_inner::<serde_json::Value>("app")
                .expect("the section")
        });
    });

    group.finish();
}

criterion_group!(rivals, load_compare);
criterion_main!(rivals);
