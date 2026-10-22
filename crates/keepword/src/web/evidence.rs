//! One capture's evidence: what it proves, its files, its checks.

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use keepword_core::bundle::Status;
use keepword_store::Record;
use maud::html;

use super::view::{self, Ctx, Nav};
use super::{Ui, WebError, WebResult, not_found};
use crate::{Node, method_name};

fn record(ui: &Ui, id: &str) -> WebResult<Record> {
    let d = id.parse().map_err(|_| not_found("This record"))?;
    ui.node
        .store
        .get(&d)?
        .ok_or_else(|| not_found("This record"))
}

pub(super) async fn page(
    State(ui): State<Ui>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> WebResult<Response> {
    let rec = record(&ui, &id)?;
    let report = Node::verify_bundle(&ui.node.bundle(&rec, true)?);
    let summary = report.summary();
    let a = &rec.signed.attestation;
    let raw_kept = ui.node.store.blobs.has(&a.body_hash);
    let size = ui.node.store.with_merkle(|m| m.len())?;
    let body = html! {
        nav.crumbs { a href=(view::page_link(&a.url)) { "← " (view::address(&a.url)) } }
        div.head {
            div {
                h1 { "Evidence record" }
                div.sub { (view::address(&a.url)) " · captured " (view::absolute(a.fetched_at_ms)) }
            }
        }
        div.card {
            @match summary.strength {
                Some(s) => { p.verdict.good { "✓ Verified: " (s.label()) } }
                None => { p.verdict.bad { "✗ This record doesn't verify" } }
            }
            @if summary.strength.is_some() {
                div.two {
                    div {
                        h3 { "It shows" }
                        ul { @for l in &summary.shows { li { (l) } } }
                    }
                    div {
                        h3.muted { "It doesn't show" }
                        ul.muted { @for l in &summary.does_not_show { li { (l) } } }
                    }
                }
            }
            div.actions {
                a.btn.primary href={ "/a/" (rec.id) "/bundle.json" } download { "Download the evidence bundle" }
                @if raw_kept { a.btn href={ "/a/" (rec.id) "/warc" } download { "Download as WARC" } }
            }
            p.muted.small {
                "Anyone can check a bundle without trusting this node: "
                code { "keepword verify --bundle FILE" }
            }
        }
        section {
            h2 { "The capture" }
            dl.facts.card {
                dt { "Address" } dd.break { (a.url) }
                @if a.final_url != a.url {
                    dt { "Ended up at" } dd.break { (a.final_url) " (" (a.redirects.len()) " redirects)" }
                }
                dt { "Captured" } dd { (view::absolute(a.fetched_at_ms)) " (" (method_name(a.method)) ")" }
                dt { "Answer" } dd { "HTTP " (a.status) " · " (a.content_type.as_deref().unwrap_or("no type")) " · " (a.body_len) " bytes" }
                @if let Some(ip) = a.server_ip { dt { "Server" } dd.mono { (ip) } }
                @if let Some(c) = a.cert_sha256 {
                    @let hex: String = c.iter().map(|x| format!("{x:02x}")).collect();
                    dt { "Certificate" } dd { a href={ "https://crt.sh/?sha256=" (hex) } rel="noreferrer noopener" target="_blank" { "look it up on crt.sh ↗" } }
                }
                dt { "In the log" } dd { "record " (rec.leaf_index + 1) " of " (size) }
                dt { "Record ID" } dd.mono.break { (rec.id) }
            }
        }
        section {
            h2 { "Checks" }
            ul.list.checks {
                @for c in &report.checks {
                    @let (cls, label) = match c.status {
                        Status::Pass => ("ok", "✓"),
                        Status::Fail => ("bad", "✗"),
                        Status::Skip => ("skip", "–"),
                    };
                    li.row {
                        span class={ "mark " (cls) } { (label) }
                        span.grow { b { (c.name) } div.sub { (c.detail) } }
                    }
                }
            }
        }
        details.card {
            summary { b { "Signed statement" } span.muted.small { " (what the witness signed)" } }
            pre { (serde_json::to_string_pretty(&rec.signed)?) }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Pages, "Evidence record", body))
}

fn download(content_type: &str, name: String, body: impl IntoResponse) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response()
}

pub(super) async fn bundle_json(
    State(ui): State<Ui>,
    Path(id): Path<String>,
) -> WebResult<Response> {
    let rec = record(&ui, &id)?;
    let json = serde_json::to_string_pretty(&ui.node.bundle(&rec, true)?)?;
    Ok(download(
        "application/json",
        format!("keepword-{}.json", rec.id.short()),
        json,
    ))
}

pub(super) async fn warc(State(ui): State<Ui>, Path(id): Path<String>) -> WebResult<Response> {
    let rec = record(&ui, &id)?;
    let a = &rec.signed.attestation;
    let (Some(h), Some(body)) = (
        ui.node.store.blobs.get(&a.headers_hash)?,
        ui.node.store.blobs.get(&a.body_hash)?,
    ) else {
        return Err(WebError(
            StatusCode::GONE,
            "This node didn't keep the raw page.".into(),
        ));
    };
    Ok(download(
        "application/warc",
        format!("keepword-{}.warc", rec.id.short()),
        keepword_capture::warc::export(&rec.signed, &h, &body),
    ))
}

pub(super) async fn tree_head(State(ui): State<Ui>) -> WebResult<Response> {
    let h = ui
        .node
        .store
        .latest_tree_head()?
        .ok_or_else(|| not_found("The log"))?;
    Ok(axum::Json(h).into_response())
}
