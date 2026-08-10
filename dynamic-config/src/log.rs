//! Diagnostics from the places with nobody to return an error to.
//!
//! A file watcher on its own thread, a remote watch loop, a cache that could
//! not be written, a start that fell back to yesterday's configuration: each
//! reports and carries on. With the `tracing` feature they become proper
//! events; without it, stderr — which is the right default for a library that
//! must not choose a logging framework for its users.

#[cfg(feature = "tracing")]
macro_rules! info {
    ($($arg:tt)*) => { ::tracing::info!(target: "dynamic_config", $($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! info {
    ($($arg:tt)*) => { ::std::eprintln!("[dynamic-config] {}", ::std::format!($($arg)*)) };
}

#[cfg(feature = "tracing")]
macro_rules! warning {
    ($($arg:tt)*) => { ::tracing::warn!(target: "dynamic_config", $($arg)*) };
}

#[cfg(not(feature = "tracing"))]
macro_rules! warning {
    ($($arg:tt)*) => { ::std::eprintln!("[dynamic-config] {}", ::std::format!($($arg)*)) };
}

pub(crate) use {info, warning};
