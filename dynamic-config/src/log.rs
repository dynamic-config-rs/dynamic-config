//! Diagnostics from the places with nobody to return an error to.
//!
//! A file watcher on its own thread, a remote watch loop, a cache that could
//! not be written, a start that fell back to yesterday's configuration: each
//! reports and carries on. Where the report lands is layered, most specific
//! wins:
//!
//! 1. **The `tracing` feature**, when compiled in, takes everything: the
//!    lines become events under the `dynamic_config` target and the layers
//!    below never run. Filtering is the subscriber's business.
//! 2. **An installed sink** ([`set_log_sink`]) receives every line that
//!    passes the level ([`set_log_level`]). This is the runtime path — it
//!    is how the language bindings hand these lines to `logging` and to a
//!    JavaScript callback, from a wheel that cannot flip a cargo feature.
//! 3. **The `log` feature**, when compiled in, forwards to the `log` crate's
//!    global logger.
//! 4. **stderr**, prefixed `[dynamic-config]` — the default, unchanged since
//!    0.1: a library must not choose a logging framework for its users, and
//!    silence would hide a watcher that is failing every reload.

use core::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwapOption;

/// How loud the engine's own diagnostics are, for every path except a
/// compiled-in `tracing` subscriber (which does its own filtering).
///
/// Ordered: a level admits itself and everything louder, so
/// [`LogLevel::Info`] — the default, matching what the engine has always
/// printed — admits warnings too, and [`LogLevel::Off`] admits nothing.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Nothing at all, the sink included.
    Off = 0,
    /// Only what went wrong: failed reloads, watcher errors, a cache that
    /// could not be written, a start that fell back to the last known good
    /// configuration.
    Warn = 1,
    /// The above, plus one line per successful reload. The default.
    Info = 2,
}

static LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Where a diagnostic line goes when the `tracing` feature is not compiled
/// in: the level it was emitted at, and the formatted line without the
/// `[dynamic-config]` prefix (the sink knows who it installed).
pub type LogSink = dyn Fn(LogLevel, &str) + Send + Sync;

static SINK: ArcSwapOption<Box<LogSink>> = ArcSwapOption::const_empty();

/// Sets how loud the engine's diagnostics are. Process-wide, effective
/// immediately, cheap enough to call per test.
///
/// Under a compiled-in `tracing` subscriber this is a no-op: events are
/// always emitted and the subscriber filters.
pub fn set_log_level(level: LogLevel) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Routes every diagnostic line that passes the level to `sink`, instead
/// of stderr or the `log` crate.
///
/// The contract, and it is load-bearing:
///
/// - **It is called on engine threads** — the watcher, a remote poll loop,
///   whatever thread called `reload()`. It must not block: a sink that
///   waits stalls reloads.
/// - **It must not call back into the engine.** Some lines are emitted
///   mid-transition; re-entrancy is not promised anywhere.
/// - One sink per process. Installing a second replaces the first;
///   [`clear_log_sink`] restores the default.
pub fn set_log_sink(sink: impl Fn(LogLevel, &str) + Send + Sync + 'static) {
    SINK.store(Some(Arc::new(Box::new(sink))));
}

/// Removes an installed sink: lines fall back to the `log` crate when that
/// feature is compiled in, and to stderr otherwise.
pub fn clear_log_sink() {
    SINK.store(None);
}

/// The runtime dispatch, shared by both macros' non-`tracing` arms.
///
/// Wait-free on the hot path: one atomic load for the level and one
/// arc-swap load for the sink, and the line is not even formatted when the
/// level refuses it (the macros pass `format_args!` through).
///
/// Compiled out under `tracing`, whose macros never call it — the sink
/// and level still exist there as public API, inert by documented design.
#[cfg(not(feature = "tracing"))]
pub(crate) fn emit(level: LogLevel, line: core::fmt::Arguments<'_>) {
    if (level as u8) > LEVEL.load(Ordering::Relaxed) {
        return;
    }

    if let Some(sink) = SINK.load_full() {
        sink(level, &line.to_string());

        return;
    }

    #[cfg(feature = "log")]
    {
        let mapped = match level {
            LogLevel::Warn => ::log::Level::Warn,
            _ => ::log::Level::Info,
        };
        ::log::log!(target: "dynamic_config", mapped, "{line}");
    }

    #[cfg(not(feature = "log"))]
    {
        ::std::eprintln!("[dynamic-config] {line}");
    }
}

#[cfg(feature = "tracing")]
macro_rules! info {
    ($($arg:tt)*) => { ::tracing::info!(target: "dynamic_config", $($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! info {
    ($($arg:tt)*) => {
        crate::log::emit(crate::log::LogLevel::Info, ::std::format_args!($($arg)*))
    };
}

#[cfg(feature = "tracing")]
macro_rules! warning {
    ($($arg:tt)*) => { ::tracing::warn!(target: "dynamic_config", $($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! warning {
    ($($arg:tt)*) => {
        crate::log::emit(crate::log::LogLevel::Warn, ::std::format_args!($($arg)*))
    };
}

pub(crate) use {info, warning};
