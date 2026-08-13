//! What a reload says about itself, and the numbers a scrape reads.
//!
//! Two halves, behind two features, because they answer to two different
//! consumers and neither should drag in the other:
//!
//! - **`tracing`** — an install and a refusal each emit a structured record
//!   on the reload path, carrying the [reload reason](crate::ReloadReason),
//!   the generation, and (on a refusal) the
//!   [`ErrorKind`](crate::ErrorKind) and key path; a fetch from a remote
//!   store runs in a `dynamic_config.fetch` span with an event inside it
//!   carrying the outcome. There is nothing on the read path: reads are an
//!   atomic load and stay one.
//! - **`telemetry`** — [`Exposition`], which renders a [`ConfigStatus`] and a
//!   [`RemoteStatus`] as Prometheus text. **No
//!   dependency at all**, which is the whole point.
//!
//! # This crate does not pick a metrics ecosystem
//!
//! A library that depends on `prometheus` picks a fight with every
//! application that chose `metrics`, or OpenTelemetry, or nothing. So it
//! depends on none of them. What it offers instead is the numbers —
//! [`ConfigStatus`] is a handful of atomic loads and no
//! I/O, so an exporter may call it per scrape — and one rendering of them,
//! in a text format that is a wire encoding rather than a crate.
//!
//! An application already running `metrics` or an OpenTelemetry SDK reads
//! `status()` in its own recorder and never touches this module; an
//! application that wants a `/metrics` handler and nothing else uses
//! [`Exposition`] and pulls in no exporter at all. Spans reach
//! OpenTelemetry the way every other crate's do, through
//! `tracing-opentelemetry`, which is the application's dependency and not
//! this crate's.
//!
//! # Nothing here can carry a value
//!
//! A metric label is a diagnostic surface like a log line, and a scrape
//! endpoint is usually the *least* guarded one a process has. So the same
//! rule holds, one notch tighter: an event field may name a key path — that
//! is what makes a failure actionable — and **a metric label may not even do
//! that**. A path is unbounded cardinality as well as a disclosure, and a
//! series named after a key is how one badly-labelled counter becomes a
//! million of them.

#[cfg(feature = "telemetry")]
use std::fmt;

#[cfg(feature = "telemetry")]
use crate::reload::ConfigStatus;
#[cfg(feature = "tracing")]
use crate::reload::ReloadReason;
#[cfg(feature = "telemetry")]
use crate::remote::RemoteStatus;

// ---------------------------------------------------------------------------
// The `tracing` half: two records on the reload path, and none anywhere else.
// ---------------------------------------------------------------------------

/// The span an install runs in, plus the event that reports it.
///
/// Entered around the reload hooks so that whatever a hook logs is
/// attributed to the reload that ran it, and dropped when the caller's
/// guard goes out of scope.
///
/// `config` is [`std::any::type_name`] of the configuration type: the only
/// name a cell has — it is the storage for a *type*, and the section key
/// belongs to the builder that loaded it. It is a diagnostic string, not an
/// identifier to match on.
#[cfg(feature = "tracing")]
pub(crate) fn installed<T: ?Sized>(
    reason: &ReloadReason,
    generation: u64,
) -> tracing::span::EnteredSpan {
    let span = tracing::info_span!(
        target: "dynamic_config",
        "dynamic_config.reload",
        config = std::any::type_name::<T>(),
        // The category, never `FileChanged`'s path: this field is copied
        // into metric labels by everything that consumes it.
        reason = reason.as_str(),
        generation = generation,
        outcome = "installed",
    );
    let entered = span.entered();

    // Inside the span, so a subscriber that prints events prints one line
    // per install rather than leaving the span to be inferred.
    tracing::info!(
        target: "dynamic_config",
        "a configuration snapshot was installed"
    );

    entered
}

/// The event for a reload that installed nothing.
///
/// An event rather than a span: nothing follows it that could be its child.
/// The key path is here and the value is not — the same line the rest of
/// this crate draws, and `tests/security.rs` is where it is enforced.
#[cfg(feature = "tracing")]
pub(crate) fn refused<T: ?Sized>(error: &crate::Error) {
    tracing::warn!(
        target: "dynamic_config",
        config = std::any::type_name::<T>(),
        outcome = "rejected",
        error.kind = error.kind().as_str(),
        // A key path, which a diagnostic may name. Recorded as a string
        // rather than through `Display`, so a structured subscriber gets a
        // string field instead of a rendered one.
        error.path = error.path().as_str(),
        "a reload installed nothing; the previous snapshot is still serving"
    );
}

/// The span a fetch from a remote store runs in.
///
/// Opened *before* the round trip rather than after it, because the whole
/// reason to span a fetch is the duration a subscriber reads off it. The
/// outcome is filled in when there is one, and an event carrying the same
/// outcome is emitted inside the span — a `Span::record` is invisible to a
/// subscriber that only prints events, and the outcome is the field anybody
/// asserts on.
///
/// **There is no field naming the store.** The only string a
/// [`Remote`](crate::Remote) can produce for itself is
/// [`describe`](crate::Remote::describe), which is the store's URL, and a
/// store URL routinely embeds `user:password@host`. What identifies a fetch
/// is the span it is nested in — the caller's own — and what identifies a
/// *series* is the label whoever renders the [`Exposition`] chose.
#[cfg(feature = "tracing")]
pub(crate) fn fetching() -> tracing::span::EnteredSpan {
    fetching_span().entered()
}

/// [`fetching`], unentered — for the async path, where the guard would have
/// to be held across an await point and `EnteredSpan` is `!Send`.
#[cfg(all(feature = "tracing", feature = "async"))]
pub(crate) fn fetching_async() -> tracing::Span {
    fetching_span()
}

#[cfg(feature = "tracing")]
fn fetching_span() -> tracing::Span {
    tracing::info_span!(
        target: "dynamic_config",
        "dynamic_config.fetch",
        outcome = tracing::field::Empty,
    )
}

/// A fetch that came back with a document.
#[cfg(feature = "tracing")]
pub(crate) fn fetched(span: &tracing::Span, elapsed: std::time::Duration) {
    span.record("outcome", "fetched");

    span.in_scope(|| {
        tracing::info!(
            target: "dynamic_config",
            outcome = "fetched",
            // The duration, not the document and not the store: a number of
            // milliseconds is the one thing a fetch can say about itself
            // that is neither a value nor a credential.
            duration_ms = elapsed.as_secs_f64() * 1000.0,
            "a remote store answered"
        );
    });
}

/// A fetch that came back with nothing.
///
/// `WARN`, and the category only. [`ErrorKind::Remote`](crate::ErrorKind) is
/// a store that may answer next time and [`Auth`](crate::ErrorKind::Auth) is
/// one that will not, which is the distinction a reader acts on; the message
/// is left to the `Error` the caller is handed.
#[cfg(feature = "tracing")]
pub(crate) fn fetch_failed(span: &tracing::Span, error: &crate::Error) {
    span.record("outcome", "failed");

    span.in_scope(|| {
        tracing::warn!(
            target: "dynamic_config",
            outcome = "failed",
            error.kind = error.kind().as_str(),
            "a fetch from a remote store returned nothing; the document it \
             last answered with is still in the slot"
        );
    });
}

// ---------------------------------------------------------------------------
// The `telemetry` half: Prometheus text over `ConfigStatus`.
// ---------------------------------------------------------------------------

/// Snapshots installed since the process started.
#[cfg(feature = "telemetry")]
pub const INSTALLS_TOTAL: &str = "dynamic_config_installs_total";
/// Seconds since the serving snapshot was installed.
#[cfg(feature = "telemetry")]
pub const LAST_SUCCESS_SECONDS: &str = "dynamic_config_last_success_seconds";
/// Reloads that have installed nothing since one did.
#[cfg(feature = "telemetry")]
pub const CONSECUTIVE_FAILURES: &str = "dynamic_config_consecutive_failures";
/// Seconds since the last reload that installed nothing.
#[cfg(feature = "telemetry")]
pub const LAST_FAILURE_SECONDS: &str = "dynamic_config_last_failure_seconds";
/// Always `1`; the [reload reason](crate::ReloadReason) is the label.
#[cfg(feature = "telemetry")]
pub const LAST_RELOAD_INFO: &str = "dynamic_config_last_reload_info";
/// Always `1`; the [`ErrorKind`](crate::ErrorKind) is the label.
#[cfg(feature = "telemetry")]
pub const LAST_FAILURE_INFO: &str = "dynamic_config_last_failure_info";

/// Every metric family this crate emits, in the order [`Exposition`] writes
/// them.
///
/// **These names are API.** They end up in dashboards and alert rules, so a
/// rename is a breaking change and belongs in the changelog under
/// *Changed*.
#[cfg(feature = "telemetry")]
pub const METRIC_NAMES: [&str; 6] = [
    INSTALLS_TOTAL,
    LAST_SUCCESS_SECONDS,
    CONSECUTIVE_FAILURES,
    LAST_FAILURE_SECONDS,
    LAST_RELOAD_INFO,
    LAST_FAILURE_INFO,
];

/// `1` when the store answered the last time it was asked, `0` when it did
/// not, and **absent** before it has been asked at all.
#[cfg(feature = "telemetry")]
pub const REMOTE_UP: &str = "dynamic_config_remote_up";
/// Documents a remote source has handed over since the process started.
#[cfg(feature = "telemetry")]
pub const REMOTE_FETCHES_TOTAL: &str = "dynamic_config_remote_fetches_total";
/// Seconds since a remote source last handed one over.
#[cfg(feature = "telemetry")]
pub const REMOTE_LAST_FETCH_SECONDS: &str = "dynamic_config_remote_last_fetch_seconds";
/// How long the last *pulled* fetch took, in seconds.
///
/// A gauge of one measurement rather than a histogram, and named for it: a
/// dashboard that met `dynamic_config_remote_fetch_duration_seconds` would
/// reach for `histogram_quantile`, and there are no buckets here to give it.
/// Rendering percentiles would mean keeping a reservoir per source, which is
/// state a library has no business choosing the shape of — an application
/// that wants them times its own `refresh_remote` call in its own recorder.
#[cfg(feature = "telemetry")]
pub const REMOTE_LAST_FETCH_DURATION_SECONDS: &str =
    "dynamic_config_remote_last_fetch_duration_seconds";
/// Fetches that returned nothing since one returned a document.
#[cfg(feature = "telemetry")]
pub const REMOTE_CONSECUTIVE_FAILURES: &str = "dynamic_config_remote_consecutive_failures";
/// Always `1`; the [`ErrorKind`](crate::ErrorKind) is the label.
#[cfg(feature = "telemetry")]
pub const REMOTE_LAST_FAILURE_INFO: &str = "dynamic_config_remote_last_failure_info";

/// Every metric family a [`RemoteStatus`] renders as, in the order
/// [`Exposition`] writes them.
///
/// **These names are API**, on the same terms as [`METRIC_NAMES`]. Separate
/// from that array rather than appended to it, because the two describe
/// different things and a process with no remote source emits only the
/// first.
#[cfg(feature = "telemetry")]
pub const REMOTE_METRIC_NAMES: [&str; 6] = [
    REMOTE_UP,
    REMOTE_FETCHES_TOTAL,
    REMOTE_LAST_FETCH_SECONDS,
    REMOTE_LAST_FETCH_DURATION_SECONDS,
    REMOTE_CONSECUTIVE_FAILURES,
    REMOTE_LAST_FAILURE_INFO,
];

/// One or more configurations' [`ConfigStatus`], as
/// Prometheus text.
///
/// Built per scrape and thrown away: every sample comes from
/// [`status`](crate::ConfigCell::status), which is atomic loads and no I/O,
/// so there is no state here worth keeping between scrapes and nothing to
/// go stale.
///
/// # What it emits
///
/// | Name | Type | Extra label | Value |
/// |---|---|---|---|
/// | `dynamic_config_installs_total` | counter | | installs since the process started |
/// | `dynamic_config_last_success_seconds` | gauge | | seconds since the serving snapshot landed |
/// | `dynamic_config_consecutive_failures` | gauge | | failures since the last install; **zero is healthy** |
/// | `dynamic_config_last_failure_seconds` | gauge | | seconds since the last failure |
/// | `dynamic_config_last_reload_info` | gauge | `reason` | `1` |
/// | `dynamic_config_last_failure_info` | gauge | `kind` | `1` |
///
/// The two `_seconds` families and the two failure families are **absent**
/// rather than zero where the fact does not exist yet: a configuration that
/// has never been installed has no staleness, and zero would read as
/// "installed just now", which is the opposite. `last_success_seconds` is
/// the one an alert is written against — *this service's configuration has
/// been stale for an hour* is the page that matters, and nothing else here
/// implies it.
///
/// A [`RemoteStatus`] added through [`add_remote`](Self::add_remote) renders
/// as six more families, and the labels are the caller's in exactly the same
/// way:
///
/// | Name | Type | Extra label | Value |
/// |---|---|---|---|
/// | `dynamic_config_remote_up` | gauge | | `1` if the store answered last time, `0` if not |
/// | `dynamic_config_remote_fetches_total` | counter | | documents the store has handed over |
/// | `dynamic_config_remote_last_fetch_seconds` | gauge | | seconds since it last handed one over |
/// | `dynamic_config_remote_last_fetch_duration_seconds` | gauge | | how long the last pull took |
/// | `dynamic_config_remote_consecutive_failures` | gauge | | fetches that returned nothing since one did |
/// | `dynamic_config_remote_last_failure_info` | gauge | `kind` | `1` |
///
/// `remote_up` is **absent** before the first fetch, on the same principle
/// as the two `_seconds` families: a source that has been installed and
/// never asked is not down, and a `0` that means "not yet" is a page nobody
/// should be woken by.
///
/// # Cardinality
///
/// Bounded by the caller, and stated so it can be checked. For `C`
/// configurations and `R` remote sources added — `R ≤ C`, because a
/// configuration type has one [`Remote`](crate::Remote):
///
/// | | per scrape | over a process's life |
/// |---|---|---|
/// | a `ConfigStatus` | `6 × C` | `(4 + 5 + 10) × C = 19 × C` |
/// | a `RemoteStatus` | `6 × R` | `(5 + 10) × R = 15 × R` |
///
/// so **at most `6 × C + 6 × R ≤ 12 × C` series in a scrape** and `34 × C`
/// distinct series in total. The label sets come from fixed enums — five
/// [reload reasons](crate::ReloadReason) and ten
/// [error kinds](crate::ErrorKind) — and `C` is the number of
/// configurations a process has, which is a handful.
///
/// A per-*store* series multiplies by a bounded number, which is why it
/// exists; a per-*key* one would not, which is why there is no method here
/// that could make one.
///
/// **No key path, file name, store key or configured value is ever a
/// label**, and there is no method here that could make one: labels are the
/// caller's own, and everything derived from a status is a count, a
/// duration or a fixed enum's name.
///
/// # Example
///
/// ```
/// use dynamic_config::{telemetry::Exposition, ConfigCell};
///
/// static PORT: ConfigCell<u16> = ConfigCell::new();
/// PORT.store(8080);
///
/// let mut exposition = Exposition::new();
/// exposition.add("listener", &PORT.status());
///
/// let rendered = exposition.render();
/// assert!(rendered.contains(r#"dynamic_config_installs_total{config="listener"} 1"#));
/// ```
#[cfg(feature = "telemetry")]
#[cfg_attr(docsrs, doc(cfg(feature = "telemetry")))]
#[derive(Debug, Clone, Default)]
pub struct Exposition {
    entries: Vec<Entry>,
    remotes: Vec<RemoteEntry>,
}

#[cfg(feature = "telemetry")]
#[derive(Debug, Clone)]
struct Entry {
    /// The caller's labels, already escaped and joined — `a="b",c="d"` —
    /// so a family that adds one of its own only has to append.
    labels: String,
    status: ConfigStatus,
}

#[cfg(feature = "telemetry")]
#[derive(Debug, Clone)]
struct RemoteEntry {
    labels: String,
    status: RemoteStatus,
}

#[cfg(feature = "telemetry")]
impl Exposition {
    /// An empty exposition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one configuration, labelled `config="…"`.
    ///
    /// The name is the caller's: a section key, a service name, whatever
    /// distinguishes this configuration from the others in the process.
    /// **Not** a path or anything derived from the document.
    pub fn add(&mut self, config: &str, status: &ConfigStatus) {
        self.add_with(&[("config", config)], status);
    }

    /// [`add`](Self::add), with labels of the caller's choosing.
    ///
    /// For a process whose configurations need two dimensions rather than
    /// one — a config server's application and profile, say. Label names
    /// are sanitised to Prometheus's `[a-zA-Z_][a-zA-Z0-9_]*` and values are
    /// escaped, so no caller can break out of the exposition; the
    /// *cardinality* of what is passed stays the caller's own decision, and
    /// the type's documentation says what a safe one looks like.
    pub fn add_with(&mut self, labels: &[(&str, &str)], status: &ConfigStatus) {
        self.entries.push(Entry {
            labels: render_labels(labels),
            status: status.clone(),
        });
    }

    /// Adds one remote source, labelled `config="…"`.
    ///
    /// The same name the configuration's own [`add`](Self::add) was given,
    /// so the two halves join in a query: `dynamic_config_remote_up` and
    /// `dynamic_config_last_success_seconds` for one `config` are *the store
    /// answered* and *the document installed*, which is the pair an
    /// operator is comparing.
    ///
    /// **Not the store's URL**, and there is no overload that takes one: a
    /// store URL routinely embeds `user:password@host`, so the name a series
    /// carries is the caller's own — the same rule, and the same reason, as
    /// for a key path.
    pub fn add_remote(&mut self, config: &str, status: &RemoteStatus) {
        self.add_remote_with(&[("config", config)], status);
    }

    /// [`add_remote`](Self::add_remote), with labels of the caller's
    /// choosing — as [`add_with`](Self::add_with) is to [`add`](Self::add).
    pub fn add_remote_with(&mut self, labels: &[(&str, &str)], status: &RemoteStatus) {
        self.remotes.push(RemoteEntry {
            labels: render_labels(labels),
            status: status.clone(),
        });
    }

    /// The exposition, as a Prometheus text body.
    ///
    /// The durations are measured here rather than at
    /// [`add`](Self::add), so they are as fresh as the response is.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();

        self.family(
            &mut out,
            INSTALLS_TOTAL,
            "counter",
            "Configuration snapshots installed since the process started.",
            |entry| Some((String::new(), entry.status.generation.to_string())),
        );
        self.family(
            &mut out,
            LAST_SUCCESS_SECONDS,
            "gauge",
            "Seconds since the serving configuration snapshot was installed.",
            |entry| {
                entry
                    .status
                    .stale_for()
                    .map(|elapsed| (String::new(), seconds(elapsed)))
            },
        );
        self.family(
            &mut out,
            CONSECUTIVE_FAILURES,
            "gauge",
            "Reloads that installed nothing since one did; zero is healthy.",
            |entry| Some((String::new(), entry.status.consecutive_failures.to_string())),
        );
        self.family(
            &mut out,
            LAST_FAILURE_SECONDS,
            "gauge",
            "Seconds since the last reload that installed nothing.",
            |entry| {
                entry
                    .status
                    .last_failure
                    .as_ref()
                    .map(|failure| (String::new(), seconds(failure.at.elapsed())))
            },
        );
        self.family(
            &mut out,
            LAST_RELOAD_INFO,
            "gauge",
            "Why the serving snapshot was installed; always 1.",
            |entry| {
                entry
                    .status
                    .last_reason
                    .as_ref()
                    .map(|reason| (format!("reason=\"{}\"", reason.as_str()), "1".to_owned()))
            },
        );
        self.family(
            &mut out,
            LAST_FAILURE_INFO,
            "gauge",
            "The category of the last reload that installed nothing; always 1.",
            |entry| {
                entry.status.last_failure.as_ref().map(|failure| {
                    (
                        format!("kind=\"{}\"", failure.kind.as_str()),
                        "1".to_owned(),
                    )
                })
            },
        );

        self.remote_families(&mut out);

        out
    }

    /// The six families a [`RemoteStatus`] renders as.
    ///
    /// After the reload families rather than interleaved with them: the text
    /// format wants a family's samples together, and a reader scanning a
    /// scrape wants the two questions in two blocks.
    fn remote_families(&self, out: &mut String) {
        remote_family(
            out,
            &self.remotes,
            REMOTE_UP,
            "gauge",
            "Whether the remote store answered the last time it was asked; \
             absent until it has been.",
            |status| {
                status
                    .reachable()
                    .map(|up| (String::new(), u8::from(up).to_string()))
            },
        );
        remote_family(
            out,
            &self.remotes,
            REMOTE_FETCHES_TOTAL,
            "counter",
            "Documents a remote store has handed over since the process started.",
            |status| Some((String::new(), status.fetches.to_string())),
        );
        remote_family(
            out,
            &self.remotes,
            REMOTE_LAST_FETCH_SECONDS,
            "gauge",
            "Seconds since a remote store last handed a document over.",
            |status| {
                status
                    .stale_for()
                    .map(|elapsed| (String::new(), seconds(elapsed)))
            },
        );
        remote_family(
            out,
            &self.remotes,
            REMOTE_LAST_FETCH_DURATION_SECONDS,
            "gauge",
            "How long the last pulled fetch took, in seconds.",
            |status| {
                status
                    .last_fetch_duration
                    .map(|elapsed| (String::new(), seconds(elapsed)))
            },
        );
        remote_family(
            out,
            &self.remotes,
            REMOTE_CONSECUTIVE_FAILURES,
            "gauge",
            "Fetches that returned nothing since one returned a document; zero is healthy.",
            |status| Some((String::new(), status.consecutive_failures.to_string())),
        );
        remote_family(
            out,
            &self.remotes,
            REMOTE_LAST_FAILURE_INFO,
            "gauge",
            "The category of the last fetch that returned nothing; always 1.",
            |status| {
                status.last_failure.as_ref().map(|failure| {
                    (
                        format!("kind=\"{}\"", failure.kind.as_str()),
                        "1".to_owned(),
                    )
                })
            },
        );
    }

    /// One metric family: the header, then a sample per entry that has one.
    ///
    /// Samples of a family are written together, which the text format
    /// requires — hence one pass per family rather than one per entry.
    fn family(
        &self,
        out: &mut String,
        name: &str,
        kind: &str,
        help: &str,
        sample: impl Fn(&Entry) -> Option<(String, String)>,
    ) {
        let samples: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                sample(entry).map(|(extra, value)| (join(&entry.labels, &extra), value))
            })
            .collect();

        write_family(out, name, kind, help, &samples);
    }
}

/// [`Exposition::family`], over the remote entries.
#[cfg(feature = "telemetry")]
fn remote_family(
    out: &mut String,
    entries: &[RemoteEntry],
    name: &str,
    kind: &str,
    help: &str,
    sample: impl Fn(&RemoteStatus) -> Option<(String, String)>,
) {
    let samples: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            sample(&entry.status).map(|(extra, value)| (join(&entry.labels, &extra), value))
        })
        .collect();

    write_family(out, name, kind, help, &samples);
}

/// A family's header and its samples, or nothing at all.
///
/// A family with no samples is omitted entirely — header included — rather
/// than declared and left empty: a `# TYPE` with nothing under it tells a
/// reader a metric exists and is broken, when in fact the fact it reports
/// has not happened yet.
#[cfg(feature = "telemetry")]
fn write_family(
    out: &mut String,
    name: &str,
    kind: &str,
    help: &str,
    samples: &[(String, String)],
) {
    if samples.is_empty() {
        return;
    }

    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push_str("\n# TYPE ");
    out.push_str(name);
    out.push(' ');
    out.push_str(kind);
    out.push('\n');

    for (labels, value) in samples {
        out.push_str(name);

        if !labels.is_empty() {
            out.push('{');
            out.push_str(labels);
            out.push('}');
        }

        out.push(' ');
        out.push_str(value);
        out.push('\n');
    }
}

/// A caller's labels, escaped and joined into `a="b",c="d"` — so a family
/// that adds one of its own only has to append.
///
/// Label names are coerced to Prometheus's `[a-zA-Z_][a-zA-Z0-9_]*` and
/// values are escaped, so no caller can break out of the exposition.
#[cfg(feature = "telemetry")]
fn render_labels(labels: &[(&str, &str)]) -> String {
    let mut rendered = String::new();

    for (name, value) in labels {
        if !rendered.is_empty() {
            rendered.push(',');
        }

        push_label_name(&mut rendered, name);
        rendered.push_str("=\"");
        push_label_value(&mut rendered, value);
        rendered.push('"');
    }

    rendered
}

#[cfg(feature = "telemetry")]
impl fmt::Display for Exposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// The caller's labels and a family's own, comma-joined, either side of
/// which may be empty.
#[cfg(feature = "telemetry")]
fn join(left: &str, right: &str) -> String {
    match (left.is_empty(), right.is_empty()) {
        (true, _) => right.to_owned(),
        (_, true) => left.to_owned(),
        _ => format!("{left},{right}"),
    }
}

/// A duration as seconds, with the fractional part a scrape interval can
/// actually resolve.
#[cfg(feature = "telemetry")]
fn seconds(elapsed: std::time::Duration) -> String {
    format!("{:.3}", elapsed.as_secs_f64())
}

/// A label name, coerced into `[a-zA-Z_][a-zA-Z0-9_]*`.
///
/// Coerced rather than refused: this is an exporter, and a handler that
/// panics or returns nothing because somebody named a label `pod-name` has
/// turned a diagnostic into an outage.
#[cfg(feature = "telemetry")]
fn push_label_name(out: &mut String, name: &str) {
    let start = out.len();

    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            out.push(character);
        } else {
            out.push('_');
        }
    }

    match out[start..].chars().next() {
        None => out.push('_'),
        Some(first) if first.is_ascii_digit() => out.insert(start, '_'),
        Some(_) => {}
    }
}

/// A label value, escaped as the text format requires.
///
/// The three characters that end a label value early are the three that are
/// escaped; everything else is UTF-8 and goes through. A caller cannot
/// forge a sample line by naming a section `x" } 1\n`.
#[cfg(feature = "telemetry")]
fn push_label_value(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(character),
        }
    }
}

#[cfg(all(test, feature = "telemetry"))]
mod tests {
    use super::*;

    #[test]
    fn a_label_name_is_coerced_and_a_value_is_escaped() {
        let mut name = String::new();
        push_label_name(&mut name, "pod-name");
        assert_eq!(name, "pod_name");

        let mut leading = String::new();
        push_label_name(&mut leading, "9lives");
        assert_eq!(leading, "_9lives");

        let mut empty = String::new();
        push_label_name(&mut empty, "");
        assert_eq!(empty, "_");

        let mut value = String::new();
        push_label_value(&mut value, "x\" } 1\nforged 2");
        assert_eq!(value, r#"x\" } 1\nforged 2"#);
    }

    #[test]
    fn labels_join_from_either_side_or_neither() {
        assert_eq!(join("a=\"1\"", "b=\"2\""), "a=\"1\",b=\"2\"");
        assert_eq!(join("", "b=\"2\""), "b=\"2\"");
        assert_eq!(join("a=\"1\"", ""), "a=\"1\"");
        assert_eq!(join("", ""), "");
    }
}
