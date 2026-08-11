//! A hot-reloadable axum server: edit the file, the next request sees it.
//!
//! ```text
//! cargo run -p dynamic-config --example axum_hello --features watch,json
//!
//! curl localhost:8080/
//! # then edit /tmp/dynamic-config-axum/config.json and curl again
//! ```
//!
//! The point of the example is the handler: it holds no configuration of its
//! own and takes none as state. It calls `current()`, which is an atomic load,
//! and the value it gets is whatever the last successful reload installed.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use dynamic_config::dynamic_config;
use serde::{Deserialize, Serialize};

const DIRECTORY: &str = "/tmp/dynamic-config-axum";

#[dynamic_config]
#[derive(Debug, Deserialize, Serialize)]
struct ServerConfig {
    /// Shown by the handler, so an edit is visible in the response.
    greeting: String,
    /// Changing this does *not* move the listener — see the note below.
    port: u16,
    /// Reloadable in the truest sense: read per request.
    feature_flag: bool,
}

impl ServerConfig {
    /// Runs on every load; a reload that fails it keeps the previous snapshot.
    fn validate(&self) -> Result<(), String> {
        if self.port < 1024 {
            return Err(format!(
                "port {} needs root; pick one above 1024",
                self.port
            ));
        }

        Ok(())
    }
}

/// Application state that is *not* configuration — a database pool would live
/// here. Configuration deliberately does not, because it changes underneath.
#[derive(Clone)]
struct AppState {
    started: Arc<std::time::Instant>,
}

#[derive(Serialize)]
struct Greeting {
    greeting: String,
    feature_flag: bool,
    uptime_seconds: u64,
}

async fn hello(State(state): State<AppState>) -> Json<Greeting> {
    // One load per request, reused for the whole handler: two calls could
    // straddle a reload and let one response mix two configurations.
    let config = ServerConfig::current();

    Json(Greeting {
        greeting: config.greeting.clone(),
        feature_flag: config.feature_flag,
        uptime_seconds: state.started.elapsed().as_secs(),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::path::PathBuf::from(DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("config.json"),
        r#"{
  "server": {
    "greeting": "hello from dynamic-config",
    "port": 8080,
    "feature_flag": false
  }
}
"#,
    )?;

    let sources = ServerConfig::builder("server")
        .file("/tmp/dynamic-config-axum/config.json")
        .env("APP_")
        .validate(|config| dynamic_config::Error::ok_or_invalid(config.validate()));

    sources.init()?;

    // Bound before the watcher starts, and never re-read: a listener cannot
    // move to a new port without dropping every connection on the old one, so
    // `port` is start-up configuration wearing the same struct as the rest.
    // Saying so is better than pretending otherwise.
    let address = format!("127.0.0.1:{}", ServerConfig::current().port);

    // `.detach()` because this watcher should outlive `main`'s body. Without it
    // the handle drops here and the watcher stops immediately.
    sources.watch(Duration::from_millis(250))?.detach();

    ServerConfig::on_reload(|previous, current| {
        if previous.port != current.port {
            eprintln!(
                "note: port changed {} -> {}, but the listener stays on {}",
                previous.port, current.port, previous.port
            );
        }
    });

    let state = AppState {
        started: Arc::new(std::time::Instant::now()),
    };

    let app = Router::new()
        .route("/", get(hello))
        // A readiness probe that reports what the process would load *now*,
        // without loading it — useful for catching a bad edit before it is
        // rolled out.
        .route(
            "/config/check",
            get(|| async {
                match ServerConfig::check() {
                    Ok(report) if report.is_clean() => {
                        (axum::http::StatusCode::OK, "ok".to_owned())
                    }
                    Ok(report) => (
                        axum::http::StatusCode::OK,
                        format!("{} unknown keys", report.unknown.len()),
                    ),
                    Err(error) => (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        error.to_string(),
                    ),
                }
            }),
        )
        .with_state(state);

    println!("listening on http://{address}");
    println!("edit {DIRECTORY}/config.json and curl again — no restart");

    let listener = tokio::net::TcpListener::bind(&address).await?;

    axum::serve(listener, app).await?;

    Ok(())
}
