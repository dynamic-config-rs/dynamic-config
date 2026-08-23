//! OpenTelemetry, for a program that has already chosen it.
//!
//! # What this module is, and what it deliberately is not
//!
//! It records into the **global meter** an application has already
//! configured. It does not build a pipeline, own an exporter, choose a
//! runtime or install a shutdown hook — those belong to whoever wrote
//! `main`, and a library that takes them over is a library that has to be
//! fought.
//!
//! Concretely: the `otel` feature pulls `opentelemetry`, the API crate, and
//! **not** `opentelemetry_sdk` or `opentelemetry-otlp`. If nothing has
//! installed a meter provider, every instrument here is a no-op and costs an
//! atomic load — which is exactly what should happen to telemetry nobody
//! asked to collect.
//!
//! Traces are already covered and stay where they are: the `tracing` feature
//! emits `dynamic_config.reload` and `dynamic_config.fetch` spans, and
//! `tracing-opentelemetry` in the *application* turns those into OTel spans
//! along with everything else the program traces. Emitting them twice from
//! here would produce two parents for the same work.
//!
//! # The metrics are the ones that already existed
//!
//! Every instrument below is named after a constant in
//! [`telemetry`](crate::telemetry) — the Prometheus names that are already
//! API. A dashboard built on the scrape and a dashboard built on OTLP see
//! the same series, because they are the same series.
//!
//! # Cardinality, and what an attribute may not carry
//!
//! The rule the Prometheus half follows does not bend here: **no key paths
//! and no values.** A reload reason is one of five strings, an error kind is
//! one of ten, a fingerprint is a digest with secrets masked, and a
//! configuration's name is written by whoever called this. A store's
//! description is never an attribute — a store URL routinely embeds
//! `user:password@host`.

use opentelemetry::metrics::{Counter, Meter};
use opentelemetry::{global, KeyValue};

use crate::reload::{ConfigStatus, ReloadEvent};

/// The attribute naming which configuration a sample is about.
pub const CONFIG: &str = "dynamic_config.config";
/// The attribute naming why a reload happened.
pub const REASON: &str = "dynamic_config.reason";
/// The attribute naming what kind of failure a refusal was.
pub const ERROR_KIND: &str = "dynamic_config.error_kind";
/// The attribute carrying the digest of the installed configuration.
pub const FINGERPRINT: &str = "dynamic_config.fingerprint";

/// The instruments one configuration records into.
///
/// Cheap to build and cheap to hold: an instrument from a provider nobody
/// installed does nothing at all.
#[derive(Clone)]
pub struct Recorder {
    name: &'static str,
    installs: Counter<u64>,
    failures: Counter<u64>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("config", &self.name)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// Instruments for the configuration called `name`, from the global
    /// meter.
    ///
    /// `name` is the label every sample carries, so it is the caller's to
    /// choose — usually the configuration type's name. It must be bounded:
    /// a name derived from a request, a tenant or a path is a cardinality
    /// incident waiting for a busy afternoon.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self::with_meter(name, &global::meter("dynamic-config"))
    }

    /// [`new`](Self::new), against a meter the caller already has.
    #[must_use]
    pub fn with_meter(name: &'static str, meter: &Meter) -> Self {
        Self {
            name,
            installs: meter
                .u64_counter(crate::telemetry::INSTALLS_TOTAL)
                .with_description("Snapshots installed since the process started.")
                .build(),
            failures: meter
                .u64_counter(crate::telemetry::RELOAD_FAILURES_TOTAL)
                .with_description("Reloads that installed nothing.")
                .build(),
        }
    }

    /// Records an install.
    ///
    /// The reason and the fingerprint ride along as attributes: the first is
    /// one of five strings, the second is a digest with secrets masked. Both
    /// are bounded, and neither can carry a value.
    pub fn installed<T>(&self, event: &ReloadEvent<T>, fingerprint: Option<&str>) {
        let mut attributes = vec![
            KeyValue::new(CONFIG, self.name),
            KeyValue::new(REASON, event.reason.as_str()),
        ];

        if let Some(fingerprint) = fingerprint {
            attributes.push(KeyValue::new(FINGERPRINT, fingerprint.to_owned()));
        }

        self.installs.add(1, &attributes);
    }

    /// Records a reload that installed nothing.
    ///
    /// The error's *kind* and not its message: a message names the file or
    /// the key that failed, which is unbounded and, in the case of a parse
    /// failure, is frequently the line holding the password.
    pub fn refused(&self, error: &crate::Error) {
        self.failures.add(
            1,
            &[
                KeyValue::new(CONFIG, self.name),
                KeyValue::new(ERROR_KIND, error.kind().as_str()),
            ],
        );
    }

    /// The attributes a caller's own instrument should carry to line up with
    /// these.
    ///
    /// For the gauges this module does not own — health, staleness, whatever
    /// an application already computes — so that a dashboard can join them
    /// without a translation table.
    #[must_use]
    pub fn attributes(&self, status: &ConfigStatus) -> Vec<KeyValue> {
        let mut attributes = vec![KeyValue::new(CONFIG, self.name)];

        if let Some(reason) = &status.last_reason {
            attributes.push(KeyValue::new(REASON, reason.as_str()));
        }

        attributes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no provider installed the instruments are inert, which is the
    /// property that lets this be compiled into a library without asking
    /// anybody's permission first.
    #[test]
    fn recording_without_a_provider_is_harmless() {
        let recorder = Recorder::new("Test");

        recorder.refused(&crate::Error::remote("the store is down"));

        assert!(format!("{recorder:?}").contains("Test"));
    }

    /// The one thing a `Debug` here must never do is name a store.
    #[test]
    fn the_debug_carries_the_config_name_and_nothing_else() {
        let recorder = Recorder::new("Billing");

        let rendered = format!("{recorder:?}");

        assert!(rendered.contains("Billing"));
        assert!(!rendered.contains("http"));
    }
}
