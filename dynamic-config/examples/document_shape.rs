//! Four questions about shape, answered by running them.
//!
//! ```text
//! cargo run -p dynamic-config --example document_shape --features json
//! ```
//!
//! 1. **Must a file be sectioned?** No — `.whole_document()` reads
//!    `{"host": …, "port": …}` with nothing above it.
//! 2. **A key the file has and the struct does not?** Ignored by the load,
//!    reported by `check()`, refused by `#[serde(deny_unknown_fields)]`.
//! 3. **Two files, half the struct in each?** One configuration; the later
//!    file wins where they overlap.
//! 4. **A field no source supplies?** `ErrorKind::Missing`, naming the
//!    field — unless a default or an `Option` covers it.
//!
//! Everything it reads it writes first, under `target/`, so it needs no
//! fixtures and leaves nothing in the repository.

use std::path::{Path, PathBuf};

use dynamic_config::{dynamic_config, Builder, ErrorKind};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct Server {
    host: String,
    port: u16,
}

/// The same fields, declared. `check()` reports unknown keys only when it
/// has a field list to compare against, and the attribute is what supplies
/// one: a bare `Builder` knows the type only as `T`.
#[dynamic_config]
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Declared {
    host: String,
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from("target/example-document-shape");
    std::fs::create_dir_all(&directory)?;

    one_document_no_header(&directory)?;
    a_key_the_struct_does_not_have(&directory)?;
    half_the_struct_in_each_file(&directory)?;
    a_field_nothing_supplies(&directory)?;

    Ok(())
}

/// 1. The file another tool wrote: no section header, all of it is yours.
fn one_document_no_header(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("── 1. a document with no section header ──\n");

    let file = write(
        directory,
        "server.json",
        r#"{"host": "0.0.0.0", "port": 8000}"#,
    )?;

    // The default reading — every top-level key is a section — cannot make
    // sense of this file, and says so in the terms that fix it.
    let refused = Builder::<Server>::new("server").file(&file).load();
    println!("without `.whole_document()`:\n  {}\n", refused.unwrap_err());

    let server: Server = Builder::new("server")
        .whole_document()
        .file(&file)
        .env("APP_")
        .load()?;

    println!("with `.whole_document()`:\n  {server:?}");
    println!(
        "\nThe key still names everything around the document: `APP_SERVER_PORT`\n\
         reaches `port`, the cache entry and the diagnostics are named after it.\n\
         A configuration with nothing to call itself may pass `\"\"`, and then\n\
         the environment layer is just the prefix — `APP_PORT`.\n"
    );

    Ok(())
}

/// 2. The file says more than the struct asks for.
fn a_key_the_struct_does_not_have(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("── 2. a key the struct does not name ──\n");

    let file = write(
        directory,
        "extra.json",
        r#"{"server": {"host": "0.0.0.0", "port": 8000, "hsot": "typo", "owner": "team-a"}}"#,
    )?;

    // Ignored: the file may be shared with another configuration type, with
    // another tool, or with a later version of this program.
    let server: Server = Builder::new("server").file(&file).load()?;
    println!("the load ignores it:\n  {server:?}\n");

    // Ignored is not unnoticed. `check` compares the section's top-level
    // keys with the struct's field names and guesses at a near miss —
    // through a `#[dynamic_config]` type, because the field list is what
    // the attribute knows and a bare `Builder` does not.
    let report = Declared::builder("server").file(&file).check()?;

    println!("`check()` names it:\n{report}");

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Strict {
        host: String,
        port: u16,
    }

    let error = Builder::<Strict>::new("server")
        .file(&file)
        .load()
        .expect_err("`hsot` is a field this struct does not have");

    println!("with `#[serde(deny_unknown_fields)]` the same file is refused:\n  {error}\n");

    Ok(())
}

/// 3. No single file has to be complete.
fn half_the_struct_in_each_file(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("── 3. one struct, two files ──\n");

    let base = write(directory, "base.json", r#"{"server": {"host": "0.0.0.0"}}"#)?;
    let ports = write(directory, "ports.json", r#"{"server": {"port": 8000}}"#)?;

    let server: Server = Builder::new("server").file(&base).file(&ports).load()?;
    println!("half in each:\n  {server:?}\n");

    let over = write(directory, "over.json", r#"{"server": {"port": 443}}"#)?;
    let server: Server = Builder::new("server")
        .file(&base)
        .file(&ports)
        .file(&over)
        .load()?;

    println!("where they overlap, the later file wins:\n  {server:?}");
    println!(
        "\nTables merge key by key; arrays are replaced whole, never appended.\n\
         A file that is not there is skipped, which is what makes an optional\n\
         `secrets.json` work.\n"
    );

    Ok(())
}

/// 4. The struct asks for more than the sources say.
fn a_field_nothing_supplies(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("── 4. a field no source supplies ──\n");

    let file = write(
        directory,
        "incomplete.json",
        r#"{"server": {"host": "0.0.0.0"}}"#,
    )?;

    let error = Builder::<Server>::new("server")
        .file(&file)
        .load()
        .expect_err("nothing supplies `port`");

    println!("the load fails, naming the field:");
    println!("  kind: {:?}", error.kind());
    println!("  path: {}", error.path());
    println!("  {error}\n");

    assert_eq!(error.kind(), ErrorKind::Missing);

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Tolerant {
        host: String,
        #[serde(default = "eight_thousand")]
        port: u16,
        tags: Option<Vec<String>>,
    }

    fn eight_thousand() -> u16 {
        8000
    }

    let tolerant: Tolerant = Builder::new("server").file(&file).load()?;
    println!("with a `#[serde(default)]` and an `Option`, the same file loads:\n  {tolerant:?}");
    println!(
        "\nA value the *program* can compute — but a file need not state — is\n\
         `set_default`, the layer below the files. A section no file mentions\n\
         is not a separate error: it is these missing fields.\n"
    );

    Ok(())
}

fn write(directory: &Path, name: &str, text: &str) -> std::io::Result<String> {
    let path = directory.join(name);
    std::fs::write(&path, text)?;

    Ok(path.display().to_string())
}
