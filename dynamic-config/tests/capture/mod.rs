//! A `tracing` subscriber that records what it is given, for the suites
//! that assert on the crate's structured output.
//!
//! Written here rather than pulled in: `tracing-subscriber` is a dependency
//! this crate does not need in order to read back two events, and a
//! dev-dependency is still a dependency somebody's `cargo deny` run has to
//! account for.
//!
//! # One subscriber per binary, not one per test
//!
//! `with_default` installs a subscriber for one thread, and a callsite's
//! interest is cached for the whole *process*: with twenty tests running in
//! parallel and each installing and dropping its own, whether a given event
//! is delivered depends on which thread got there first. That is the
//! shared-state mistake AGENTS.md names, wearing a different hat.
//!
//! So there is one subscriber, installed once, never dropped — and a test
//! reads back only the lines belonging to *its* configuration type, which
//! every record carries in its `config` field. One type per test keeps them
//! apart, which is the rule anyway.

#![allow(dead_code)]

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

static INSTALLED: OnceLock<Captured> = OnceLock::new();

/// The binary's capturing subscriber, installed on first use.
pub fn global() -> &'static Captured {
    INSTALLED.get_or_init(|| {
        let captured = Captured::default();

        tracing::subscriber::set_global_default(captured.clone())
            .expect("nothing else in this binary installs a subscriber");

        captured
    })
}

/// Every span and event, rendered as `kind name field=value …`.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    /// Everything recorded so far, one span or event per line.
    pub fn joined(&self) -> String {
        self.lines().join("\n")
    }

    /// The records that mention `marker` — a test's own configuration type,
    /// so a suite running in parallel does not read its neighbour's.
    pub fn about(&self, marker: &str) -> String {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(marker))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// How many records mention `marker`.
    pub fn count(&self, marker: &str) -> usize {
        self.lines()
            .iter()
            .filter(|line| line.contains(marker))
            .count()
    }

    fn lines(&self) -> Vec<String> {
        self.0.lock().expect("no test panics holding this").clone()
    }

    fn push(&self, line: String) {
        self.0
            .lock()
            .expect("no test panics holding this")
            .push(line);
    }
}

#[derive(Default)]
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        use fmt::Write as _;

        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl Subscriber for Captured {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut fields = Fields::default();
        span.record(&mut fields);
        self.push(format!("span {}{}", span.metadata().name(), fields.0));

        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.push(format!("event {}{}", event.metadata().level(), fields.0));
    }

    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}
