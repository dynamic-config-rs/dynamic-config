//! figment's parsers, behind this crate's `Reader` trait.

use crate::error::{Error, ErrorKind};
use crate::reader::Reader;
use crate::source::Format;
use crate::value::Value;

#[derive(Debug)]
pub(crate) struct Figment;

impl Reader for Figment {
    fn name(&self) -> &str {
        "figment"
    }

    fn reads(&self, format: Format) -> bool {
        // The three the backend has providers for, each behind this
        // crate's feature of the same name — which turns the
        // backend's on too, so one flag answers for both.
        match format {
            Format::Json => cfg!(feature = "json"),
            Format::Toml => cfg!(feature = "toml"),
            Format::Yaml => cfg!(feature = "yaml"),
            Format::Ini | Format::Properties | Format::Ron | Format::Json5 => false,
        }
    }

    #[allow(unused_variables)]
    fn parse(&self, text: &str, format: Format) -> Result<Value, Error> {
        // `Format` is figment's own trait for "this parser reads a
        // string", which is exactly what a reader is asked for. Both
        // are unused in a build with no format feature, where the
        // match below has nothing but its refusal.
        #[allow(unused_imports)]
        use figment::{providers::Format as _, Provider as _};

        // An `Option` and a `let ... else`, rather than a `return` inside
        // the match: with no format feature on, the match is the wildcard
        // alone and everything after a `return` there is unreachable —
        // a warning about a build shape rather than about this code.
        let provider: Option<Box<dyn figment::Provider>> = match format {
            #[cfg(feature = "json")]
            Format::Json => Some(Box::new(figment::providers::Json::string(text))),
            #[cfg(feature = "toml")]
            Format::Toml => Some(Box::new(figment::providers::Toml::string(text))),
            #[cfg(feature = "yaml")]
            Format::Yaml => Some(Box::new(figment::providers::Yaml::string(text))),
            _ => None,
        };

        let Some(provider) = provider else {
            return Err(crate::reader::unread(format));
        };

        let data = provider.data().map_err(|error| refused(&error))?;
        let dict = data.into_values().next().unwrap_or_default();

        let parsed = Value::Table(
            dict.iter()
                .map(|(key, value)| (key.clone(), super::from_figment(value)))
                .collect(),
        );

        // This backend hands a TOML datetime over as the parser's own
        // one-key table, exactly as the plain `toml` reader does — so it
        // is unwrapped the same way, and all three readers answer with
        // the text that was written.
        #[cfg(feature = "toml")]
        let parsed = if format == Format::Toml {
            crate::document::dates(parsed)
        } else {
            parsed
        };

        Ok(parsed)
    }
}

/// As the other adapter's: one line, and never the document.
fn refused(error: &figment::Error) -> Error {
    let rendered = error.to_string();
    let first = rendered.lines().next().unwrap_or_default();

    Error::new(ErrorKind::Parse, crate::loader::redacted(first))
}
