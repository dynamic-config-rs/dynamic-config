//! Serde adapters for the two units configuration is always written in.
//!
//! `timeout = 30` is ambiguous — seconds? milliseconds? — and `max_body = 67108864`
//! is unreadable. Both are usually written as `"30s"` and `"64MiB"`, which no
//! stock `Deserialize` impl accepts. These adapters do, while still accepting a
//! bare number so existing files keep working:
//!
//! ```
//! use std::time::Duration;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Config {
//!     #[serde(with = "dynamic_config::duration")]
//!     timeout: Duration,
//!     #[serde(default, with = "dynamic_config::duration::option")]
//!     grace: Option<Duration>,
//!     #[serde(with = "dynamic_config::bytes")]
//!     max_body: u64,
//! }
//! ```
//!
//! Both are strict about nonsense: an unknown unit or a bare `-` is an error
//! naming what was expected, never a silent zero.

use std::fmt;

use serde::de::{self, Unexpected, Visitor};
use serde::{Deserializer, Serializer};

/// Splits `12kb` into `(12, "kb")`, tolerating whitespace between them.
fn split_unit(text: &str) -> Result<(u64, &str), String> {
    let text = text.trim();
    let digits = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());

    if digits == 0 {
        return Err(format!("`{text}` does not start with a number"));
    }

    let value = text[..digits]
        .parse::<u64>()
        .map_err(|_| format!("`{}` is too large", &text[..digits]))?;

    Ok((value, text[digits..].trim()))
}

// ---------------------------------------------------------------------------

/// `Duration` from `"30s"`, `"1h30m"`, `"500ms"`, or a bare number of seconds.
///
/// Units: `ms`, `s`, `m`, `h`, `d`. Several may be concatenated, largest first
/// by convention but in any order in practice — `"1h30m"` is 90 minutes.
///
/// ```
/// use std::time::Duration;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Config {
///     #[serde(with = "dynamic_config::duration")]
///     timeout: Duration,
///     #[serde(default, with = "dynamic_config::duration::option")]
///     grace: Option<Duration>,
/// }
/// ```
///
/// Strict about nonsense: an unknown unit or a bare `-` is an error naming what
/// was expected, never a silent zero.
pub mod duration {
    use super::*;
    use std::time::Duration;

    /// Parses a duration string.
    ///
    /// # Errors
    ///
    /// If a component has no unit, an unknown unit, or the total overflows.
    pub fn parse(text: &str) -> Result<Duration, String> {
        let text = text.trim();

        if text.is_empty() {
            return Err("an empty string is not a duration".to_owned());
        }

        let mut total = Duration::ZERO;
        let mut rest = text;

        while !rest.is_empty() {
            let (value, tail) = split_unit(rest)?;

            let boundary = tail
                .find(|character: char| character.is_ascii_digit())
                .unwrap_or(tail.len());
            let (unit, tail) = tail.split_at(boundary);

            let component = match unit.trim() {
                "ms" => Duration::from_millis(value),
                "s" => Duration::from_secs(value),
                // `checked_mul`, as `bytes` below already does: this is
                // arithmetic on config input, and the module's promise is
                // "strict about nonsense" — a silent saturation to u64::MAX
                // seconds is nonsense accepted quietly.
                "m" => Duration::from_secs(checked(value, 60)?),
                "h" => Duration::from_secs(checked(value, 60 * 60)?),
                "d" => Duration::from_secs(checked(value, 24 * 60 * 60)?),
                "" => {
                    return Err(format!(
                        "`{value}` has no unit; expected one of ms, s, m, h, d"
                    ))
                }
                other => {
                    return Err(format!(
                        "unknown duration unit `{other}`; expected one of ms, s, m, h, d"
                    ))
                }
            };

            total = total
                .checked_add(component)
                .ok_or_else(|| format!("`{text}` is longer than a `Duration` can hold"))?;

            rest = tail;
        }

        Ok(total)
    }

    /// One component's seconds, or the error the module promises on overflow.
    fn checked(value: u64, seconds_per_unit: u64) -> Result<u64, String> {
        value
            .checked_mul(seconds_per_unit)
            .ok_or_else(|| format!("`{value}` overflows a 64-bit second count"))
    }

    struct DurationVisitor;

    impl Visitor<'_> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a duration such as \"30s\" or \"1h30m\", or a number of seconds")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Duration, E> {
            parse(value).map_err(E::custom)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<Duration, E> {
            Ok(Duration::from_secs(value))
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<Duration, E> {
            u64::try_from(value)
                .map(Duration::from_secs)
                .map_err(|_| E::invalid_value(Unexpected::Signed(value), &self))
        }
    }

    /// `#[serde(with = "dynamic_config::duration")]`
    ///
    /// # Errors
    ///
    /// If the value is neither a number nor a parseable duration string.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        deserializer.deserialize_any(DurationVisitor)
    }

    /// Round-trips as the millisecond count, so a written config stays valid.
    ///
    /// # Errors
    ///
    /// Whatever the serializer reports.
    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}ms", value.as_millis()))
    }

    /// The same, for `Option<Duration>`.
    pub mod option {
        use super::*;

        struct OptionVisitor;

        impl<'de> Visitor<'de> for OptionVisitor {
            type Value = Option<Duration>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a duration, a number of seconds, or null")
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                super::deserialize(deserializer).map(Some)
            }
        }

        /// `#[serde(with = "dynamic_config::duration::option")]`
        ///
        /// # Errors
        ///
        /// As [`duration::deserialize`](super::deserialize).
        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Duration>, D::Error> {
            deserializer.deserialize_option(OptionVisitor)
        }

        /// # Errors
        ///
        /// Whatever the serializer reports.
        pub fn serialize<S: Serializer>(
            value: &Option<Duration>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(duration) => super::serialize(duration, serializer),
                None => serializer.serialize_none(),
            }
        }
    }
}

// ---------------------------------------------------------------------------

/// A byte count from `"64MiB"`, `"1GB"`, or a bare number.
///
/// Both conventions are supported and they are *not* the same: `KiB`/`MiB`/`GiB`
/// are powers of 1024, `KB`/`MB`/`GB` powers of 1000. A bare `K`/`M`/`G` is
/// read as the binary form, which is what everyone means when they type it into
/// a config file.
///
/// ```
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Config {
///     #[serde(with = "dynamic_config::bytes")]
///     max_body: u64,
/// }
/// ```
///
/// Strict about nonsense: an unknown unit is an error naming what was expected,
/// never a silent zero.
pub mod bytes {
    use super::*;

    /// Parses a byte-size string.
    ///
    /// # Errors
    ///
    /// If the unit is unknown or the total overflows a `u64`.
    pub fn parse(text: &str) -> Result<u64, String> {
        let (value, unit) = split_unit(text)?;

        let multiplier: u64 = match unit.trim().to_ascii_lowercase().as_str() {
            "" | "b" => 1,
            "k" | "kib" => 1 << 10,
            "m" | "mib" => 1 << 20,
            "g" | "gib" => 1 << 30,
            "t" | "tib" => 1 << 40,
            "kb" => 1_000,
            "mb" => 1_000_000,
            "gb" => 1_000_000_000,
            "tb" => 1_000_000_000_000,
            other => {
                return Err(format!(
                    "unknown size unit `{other}`; expected one of B, KiB, MiB, GiB, TiB, \
                     KB, MB, GB, TB"
                ))
            }
        };

        value
            .checked_mul(multiplier)
            .ok_or_else(|| format!("`{}` overflows a 64-bit byte count", text.trim()))
    }

    struct BytesVisitor;

    impl Visitor<'_> for BytesVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a size such as \"64MiB\", or a number of bytes")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<u64, E> {
            parse(value).map_err(E::custom)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<u64, E> {
            u64::try_from(value).map_err(|_| E::invalid_value(Unexpected::Signed(value), &self))
        }
    }

    /// `#[serde(with = "dynamic_config::bytes")]`
    ///
    /// # Errors
    ///
    /// If the value is neither a number nor a parseable size string.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        deserializer.deserialize_any(BytesVisitor)
    }

    /// # Errors
    ///
    /// Whatever the serializer reports.
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(*value)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{bytes, duration};

    #[test]
    fn every_duration_unit_parses() {
        assert_eq!(
            duration::parse("500ms").unwrap(),
            Duration::from_millis(500)
        );
        assert_eq!(duration::parse("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(duration::parse("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(duration::parse("2h").unwrap(), Duration::from_secs(7_200));
        assert_eq!(duration::parse("1d").unwrap(), Duration::from_secs(86_400));
    }

    #[test]
    fn duration_components_add_up() {
        assert_eq!(
            duration::parse("1h30m").unwrap(),
            Duration::from_secs(5_400)
        );
        assert_eq!(
            duration::parse("1m 500ms").unwrap(),
            Duration::from_millis(60_500)
        );
    }

    #[test]
    fn a_duration_without_a_unit_is_an_error_not_a_guess() {
        // Ambiguous in a string: seconds or milliseconds? A bare *number* means
        // seconds, but "30" as text is a mistake worth reporting.
        let error = duration::parse("30").unwrap_err();

        assert!(error.contains("no unit"), "{error}");
    }

    #[test]
    fn an_unknown_duration_unit_lists_the_valid_ones() {
        let error = duration::parse("30w").unwrap_err();

        assert!(error.contains("unknown duration unit `w`"), "{error}");
        assert!(error.contains("ms, s, m, h, d"), "{error}");
    }

    #[test]
    fn duration_rejects_empty_and_non_numeric_input() {
        assert!(duration::parse("   ").is_err());
        assert!(duration::parse("abc").is_err());
        assert!(duration::parse("-5s").is_err());
    }

    #[test]
    fn binary_and_decimal_size_units_differ() {
        assert_eq!(bytes::parse("1KiB").unwrap(), 1_024);
        assert_eq!(bytes::parse("1KB").unwrap(), 1_000);
        assert_eq!(bytes::parse("64MiB").unwrap(), 64 * 1_024 * 1_024);
        assert_eq!(bytes::parse("1GB").unwrap(), 1_000_000_000);
    }

    #[test]
    fn a_bare_size_unit_is_binary() {
        assert_eq!(bytes::parse("1M").unwrap(), 1 << 20);
        assert_eq!(bytes::parse("512").unwrap(), 512);
        assert_eq!(bytes::parse("512B").unwrap(), 512);
    }

    #[test]
    fn size_units_are_case_insensitive() {
        assert_eq!(
            bytes::parse("64mib").unwrap(),
            bytes::parse("64MiB").unwrap()
        );
        assert_eq!(bytes::parse("1gb").unwrap(), bytes::parse("1GB").unwrap());
    }

    #[test]
    fn an_unknown_size_unit_lists_the_valid_ones() {
        let error = bytes::parse("5PB").unwrap_err();

        assert!(error.contains("unknown size unit `pb`"), "{error}");
    }

    #[test]
    fn a_size_that_overflows_is_reported_rather_than_wrapped() {
        let error = bytes::parse("100000000000TiB").unwrap_err();

        assert!(error.contains("overflows"), "{error}");
    }

    #[test]
    fn both_adapters_still_accept_a_bare_number() {
        #[derive(serde::Deserialize)]
        struct Config {
            #[serde(with = "super::duration")]
            timeout: Duration,
            #[serde(with = "super::bytes")]
            max_body: u64,
        }

        let config: Config =
            serde_json::from_str(r#"{"timeout": 30, "max_body": 1048576}"#).unwrap();

        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.max_body, 1_048_576);
    }

    #[test]
    fn the_string_forms_deserialize_through_serde() {
        #[derive(serde::Deserialize)]
        struct Config {
            #[serde(with = "super::duration")]
            timeout: Duration,
            #[serde(default, with = "super::duration::option")]
            grace: Option<Duration>,
            #[serde(with = "super::bytes")]
            max_body: u64,
        }

        let config: Config =
            serde_json::from_str(r#"{"timeout": "1h30m", "max_body": "64MiB"}"#).unwrap();

        assert_eq!(config.timeout, Duration::from_secs(5_400));
        assert_eq!(config.grace, None);
        assert_eq!(config.max_body, 64 * 1_024 * 1_024);

        let config: Config =
            serde_json::from_str(r#"{"timeout": "1s", "grace": "250ms", "max_body": 1}"#).unwrap();

        assert_eq!(config.grace, Some(Duration::from_millis(250)));
    }

    #[test]
    fn a_duration_component_that_overflows_is_an_error_not_a_saturation() {
        // Fits in u64, overflows when multiplied by 60.
        let error = duration::parse("307445734561825861m").unwrap_err();

        assert!(error.contains("overflows"), "{error}");
    }
}
