//! Alerts this node raised and heard from the network.

use axum::extract::State;
use axum::response::Response;
use keepword_core::net::{Alert, AlertKind};
use maud::{Markup, html};

use super::view::{self, Ctx, Nav};
use super::{Ui, WebResult};

pub(super) fn kind_badge(kind: AlertKind) -> Markup {
    match kind {
        AlertKind::SilentEdit => html! { span.badge.silent { "Silent edit" } },
        AlertKind::Split => html! { span.badge.status { "Different versions" } },
        AlertKind::Equivocation => html! { span.badge.silent { "Log equivocation" } },
    }
}

pub(super) async fn page(State(ui): State<Ui>, ctx: Ctx) -> WebResult<Response> {
    let node = &ui.node;
    let alerts = node.store.alerts(500)?;
    let me = node.key.public();
    // The same alert, raised on capture after capture, is one entry.
    let same = |a: &Alert, b: &Alert| {
        (a.kind, &a.url, &a.summary, a.issuer) == (b.kind, &b.url, &b.summary, b.issuer)
    };
    let mut entries = Vec::new();
    for run in alerts.chunk_by(|a, b| same(&a.body, &b.body)) {
        let (a, first) = (&run[0].body, &run[run.len() - 1].body);
        let from = if a.issuer == me {
            "this node".to_string()
        } else {
            node.store
                .peer(&a.issuer)?
                .and_then(|p| p.endpoint)
                .and_then(|e| Some(url::Url::parse(&e).ok()?.host_str()?.to_string()))
                .unwrap_or_else(|| format!("witness {}", a.issuer.short()))
        };
        let prefix = format!("{}: ", a.kind.label());
        let summary = a
            .summary
            .strip_prefix(&prefix)
            .unwrap_or(&a.summary)
            .to_string();
        entries.push(html! {
            li.row {
                div.grow {
                    (kind_badge(a.kind)) " "
                    @if let Some(u) = &a.url { a href=(view::page_link(u)) { (view::address(u)) } }
                    div.sub {
                        (summary)
                        @if run.len() > 1 { " · " (run.len()) " times since " (view::when(first.issued_at_ms)) }
                        " · from " (from)
                    }
                }
                span.muted.small { (view::when(a.issued_at_ms)) }
            }
        });
    }
    let body = html! {
        div.head { h1 { "Alerts" } }
        dl.legend.small.muted {
            dt { (kind_badge(AlertKind::SilentEdit)) } dd { "A page changed without saying so." }
            dt { (kind_badge(AlertKind::Split)) } dd { "Witnesses saw different versions of a page at the same time." }
            dt { (kind_badge(AlertKind::Equivocation)) } dd { "A witness's log showed two different histories: it can't be trusted." }
        }
        @if entries.is_empty() {
            div.empty { p { "No alerts." } p.muted { "Alerts from this node and from the witnesses it knows show up here." } }
        } @else {
            ul.list { @for e in entries { (e) } }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Alerts, "Alerts", body))
}
