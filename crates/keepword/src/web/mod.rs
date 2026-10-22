//! The web UI, where a node's operator does everything without the CLI,
//! and the watch scheduler.
//!
//! Pages are rendered on the server with `maud`, which escapes whatever it
//! interpolates: the UI shows text from third-party pages. Scripts and
//! styles are files, so the content security policy allows nothing inline,
//! and every page works without JavaScript.

mod alerts;
mod auth;
mod change;
mod evidence;
mod network;
mod overview;
mod pages;
mod settings;
mod view;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use keepword_core::{format_ms, now_ms};

use crate::Node;

pub use auth::UiAuth;

// ---------------------------------------------------------------- scheduler

/// Capture every watched URL that is due. Returns how many were attempted.
pub async fn run_due(node: &Arc<Node>, log: impl Fn(String)) -> anyhow::Result<usize> {
    let now = now_ms();
    let due: Vec<_> = node
        .store
        .watches()?
        .into_iter()
        .filter(|w| {
            // Network requests set intervals too, to any u64: one too long
            // to count in milliseconds is never due again.
            let every_ms = i64::try_from(w.every_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
            w.last_run.is_none_or(|last| now - last >= every_ms)
        })
        .collect();
    for w in &due {
        match node.capture(&w.url, w.render).await {
            Ok(o) => {
                node.store.watch_mark(&w.url, now_ms(), None)?;
                let status = match &o.change {
                    Some(c) if c.silent => format!("SILENT EDIT: {}", c.summary),
                    Some(c) => format!("changed: {}", c.summary),
                    None if o.previous.is_none() => "first capture".into(),
                    None if !o.compared => "not compared, normalized differently before".into(),
                    None => "unchanged".into(),
                };
                log(format!(
                    "{}  {}  {}  {status}",
                    format_ms(now_ms()),
                    o.record.id.short(),
                    w.url
                ));
            }
            Err(e) => {
                node.store
                    .watch_mark(&w.url, now_ms(), Some(&format!("{e:#}")))?;
                log(format!("{}  error  {}  {e:#}", format_ms(now_ms()), w.url));
            }
        }
    }
    Ok(due.len())
}

pub async fn scheduler(node: Arc<Node>, log: impl Fn(String)) -> anyhow::Result<()> {
    loop {
        // Like a failed sync, a failed round is retried: it must not stop
        // the node.
        if let Err(e) = run_due(&node, &log).await {
            log(format!("{}  watches failed: {e:#}", format_ms(now_ms())));
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

// ---------------------------------------------------------------------- web

/// Notified when the UI saved settings and asks the node to restart:
/// `keepword serve` then exits, for its supervisor to start it again.
pub static RESTART: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// The exit status `keepword serve` restarts with: systemd's
/// `Restart=on-failure` only restarts a service that failed.
pub const RESTART_STATUS: i32 = 75;

#[derive(Clone, Debug)]
pub struct Options {
    pub auth: UiAuth,
    /// The scheduler runs: watched pages are captured when due.
    pub watching: bool,
    /// A supervisor (systemd, a container runtime) starts the node again
    /// when it exits, so the UI can offer to restart it.
    pub supervised: bool,
}

#[derive(Clone)]
struct Ui {
    node: Arc<Node>,
    auth: Arc<auth::Auth>,
    opts: Arc<Options>,
}

pub fn router(node: Arc<Node>, opts: Options) -> anyhow::Result<Router> {
    let ui = Ui {
        auth: Arc::new(auth::Auth::new(&node.dir, opts.auth)?),
        node,
        opts: Arc::new(opts),
    };
    Ok(Router::new()
        .route("/", get(overview::page))
        .route("/review", post(overview::review))
        .route("/pages", get(pages::list))
        .route("/pages/add", post(pages::add))
        .route("/page", get(pages::detail))
        .route("/page/check", post(pages::check))
        .route("/page/watch", post(pages::watch))
        .route("/page/unwatch", post(pages::unwatch))
        .route("/page/purge", post(pages::purge))
        .route("/url", get(pages::old_link))
        .route("/change/{seq}", get(change::page))
        .route("/diff", get(change::diff))
        .route("/a/{id}", get(evidence::page))
        .route("/a/{id}/bundle.json", get(evidence::bundle_json))
        .route("/a/{id}/warc", get(evidence::warc))
        .route("/api/tree-head", get(evidence::tree_head))
        .route("/alerts", get(alerts::page))
        .route("/network", get(network::page))
        .route("/network/request", post(network::request))
        .route("/network/withdraw", post(network::withdraw))
        .route("/network/peer", post(network::add_peer))
        .route("/network/forget", post(network::forget_peer))
        .route("/network/sync", post(network::sync))
        .route("/settings", get(settings::page).post(settings::save))
        .route("/settings/file", post(settings::save_file))
        .route("/settings/password", post(settings::password))
        .route("/settings/restart", post(settings::restart))
        .route("/login", get(auth::login_page).post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/setup", get(auth::setup_page).post(auth::setup))
        .route("/static/app.css", get(css))
        .route("/static/app.js", get(js))
        .route("/favicon.svg", get(favicon))
        .layer(axum::middleware::from_fn_with_state(
            ui.clone(),
            auth::guard,
        ))
        .with_state(ui))
}

/// An error page. Its message is shown as text, never as markup.
struct WebError(StatusCode, String);

impl<E: std::fmt::Display> From<E> for WebError {
    fn from(e: E) -> Self {
        WebError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let title = match self.0 {
            StatusCode::NOT_FOUND => "Not found",
            StatusCode::GONE => "Not kept",
            s if s.is_client_error() => "Can't do that",
            _ => "Something went wrong",
        };
        (self.0, view::bare(title, &self.1)).into_response()
    }
}

type WebResult<T> = Result<T, WebError>;

fn not_found(what: &str) -> WebError {
    WebError(StatusCode::NOT_FOUND, format!("{what} not found."))
}

fn bad_request(msg: impl Into<String>) -> WebError {
    WebError(StatusCode::BAD_REQUEST, msg.into())
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Pages link them with the version in the query string.
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        body,
    )
        .into_response()
}

async fn css() -> Response {
    asset("text/css; charset=utf-8", include_str!("assets/app.css"))
}

async fn js() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("assets/app.js"),
    )
}

/// Keepword's seal: the tab icon, and the mark in the page header.
async fn favicon() -> Response {
    asset("image/svg+xml", include_str!("assets/favicon.svg"))
}
