//! The `config` crate's parsers, behind this crate's `Reader` trait.
//!
//! Reads two formats nothing here parses — RON and JSON5 — and reads YAML
//! through `yaml-rust2`, which is maintained where this crate's own
//! `serde_yaml` is archived.

use crate::error::Error;
use crate::reader::Reader;
use crate::source::Format;
use crate::value::Value;

#[derive(Debug)]
pub(crate) struct ConfigRs;

impl Reader for ConfigRs {
    fn name(&self) -> &str {
        "config-rs"
    }

    fn reads(&self, format: Format) -> bool {
        dialect(format).is_some()
    }

    fn parse(&self, text: &str, format: Format) -> Result<Value, Error> {
        let Some(dialect) = dialect(format) else {
            return Err(crate::reader::unread(format));
        };

        let source = config_rs::File::from_str(text, dialect);
        let collected =
            config_rs::Source::collect(&source).map_err(|error| super::error::unparsed(&error))?;

        Ok(Value::Table(
            collected
                .into_iter()
                .map(|(key, value)| (key, super::from_config_rs(&value)))
                .collect(),
        ))
    }
}

/// This crate's format, as the backend's — `None` when the backend was
/// built without that parser.
#[allow(unreachable_patterns, unused_variables)]
fn dialect(format: Format) -> Option<config_rs::FileFormat> {
    match format {
        #[cfg(feature = "json")]
        Format::Json => Some(config_rs::FileFormat::Json),
        #[cfg(feature = "toml")]
        Format::Toml => Some(config_rs::FileFormat::Toml),
        #[cfg(feature = "yaml")]
        Format::Yaml => Some(config_rs::FileFormat::Yaml),
        #[cfg(feature = "ini")]
        Format::Ini => Some(config_rs::FileFormat::Ini),
        #[cfg(feature = "ron")]
        Format::Ron => Some(config_rs::FileFormat::Ron),
        #[cfg(feature = "json5")]
        Format::Json5 => Some(config_rs::FileFormat::Json5),
        // `.properties` is this crate's own, and the backend has no
        // parser for it at all.
        _ => None,
    }
}
