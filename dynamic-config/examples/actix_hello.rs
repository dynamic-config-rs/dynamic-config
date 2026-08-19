//! A hot-reloadable Actix Web server: edit the file, the next request sees it.
//!
//! ```text
//! cargo run -p dynamic-config --example actix_hello --features watch,json
//!
//! curl localhost:8081/
//! # then edit /tmp/dynamic-config-actix/config.json and curl again
//! ```
//!
//! The same shape as the axum example, and deliberately so: configuration is
//! not application state, it is read where it is used. Actix runs handlers on
//! several worker threads, which makes the point sharper — `current()` is an
//! atomic load, so every worker sees the new snapshot without a lock between
//! them and without anyone being told to reload.

use std::time::Duration;

use actix_web::{get, App, HttpResponse, HttpServer, Responder};
use dynamic_config::dynamic_config;
use serde::{Deserialize, Serialize};

const DIRECTORY: &str = "/tmp/dynamic-config-actix";

#[dynamic_config]
#[derive(Debug, Deserialize, Serialize)]
struct ServerConfig {
    greeting: String,
    port: u16,
    /// Applied per request, so a change takes effect without a restart.
    max_items: usize,
}

impl ServerConfig {
    fn validate(&self) -> Result<(), String> {
        if self.max_items == 0 {
            return Err("max_items = 0 would serve nothing".to_owned());
        }

        Ok(())
    }
}

#[derive(Serialize)]
struct Greeting {
    greeting: String,
    items: Vec<usize>,
}

#[get("/")]
async fn hello() -> impl Responder {
    // One load per request, reused for the whole handler: two calls could
    // straddle a reload and let one response mix two configurations.
    let config = ServerConfig::current();

    HttpResponse::Ok().json(Greeting {
        greeting: config.greeting.clone(),
        items: (0..config.max_items).collect(),
    })
}

#[get("/config/check")]
async fn check() -> impl Responder {
    // Reports what the process *would* load, without loading it — for catching
    // a bad edit before it is rolled out.
    match ServerConfig::check() {
        Ok(report) if report.is_clean() => HttpResponse::Ok().body("ok"),
        Ok(report) => HttpResponse::Ok().body(format!("{} unknown keys", report.unknown.len())),
        Err(error) => HttpResponse::ServiceUnavailable().body(error.to_string()),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let directory = std::path::PathBuf::from(DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("config.json"),
        r#"{
  "server": {
    "greeting": "hello from dynamic-config",
    "port": 8081,
    "max_items": 3
  }
}
"#,
    )?;

    let sources = ServerConfig::builder("server")
        .file("/tmp/dynamic-config-actix/config.json")
        .env("APP_")
        .validate(|config| dynamic_config::Error::ok_or_invalid(config.validate()));

    sources.init().map_err(std::io::Error::other)?;

    // Read once, before the watcher starts: a listener cannot move to a new
    // port without dropping every connection on the old one, so `port` is
    // start-up configuration wearing the same struct as the rest.
    let port = ServerConfig::current().port;

    // `.detach()` because this watcher should outlive `main`'s body.
    sources.watch(Duration::from_millis(250))?.detach();

    println!("listening on http://127.0.0.1:{port}");
    println!("edit {DIRECTORY}/config.json and curl again — no restart");

    // Note what is *not* here: no `web::Data<ServerConfig>`. Handing a snapshot
    // to the app factory would freeze it at start-up, and every worker would
    // then serve the configuration that existed when it was built.
    HttpServer::new(|| App::new().service(hello).service(check))
        .bind(("127.0.0.1", port))?
        .run()
        .await
}
