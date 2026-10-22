//! A change: what it is, what changed, and the evidence on both sides.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use keepword_core::Digest;
use keepword_core::quorum::comparable;
use keepword_normalize::diff::{self, Change, Disclosure};
use keepword_store::{ChangeRow, Record};
use maud::{Markup, html};

use super::auth::redirect;
use super::view::{self, Ctx, Nav};
use super::{Ui, WebResult, not_found};
use crate::Node;

/// Two captures and what changed between them.
pub(super) struct Loaded {
    pub from: Record,
    pub to: Record,
    /// `None` when the node didn't keep the text of one of them.
    pub change: Option<Change>,
    /// The newer version's title.
    pub title: Option<String>,
}

pub(super) fn load(node: &Node, from: &Digest, to: &Digest) -> WebResult<Loaded> {
    let get = |id: &Digest| -> WebResult<Record> {
        node.store.get(id)?.ok_or_else(|| not_found("A capture"))
    };
    let (from, to) = (get(from)?, get(to)?);
    let (a, b) = (
        node.normalized_text(&from.signed.attestation)?,
        node.normalized_text(&to.signed.attestation)?,
    );
    let title = b.as_deref().and_then(view::title_of);
    let change = match (a, b) {
        (Some(a), Some(b)) => diff::diff(&a, &b),
        _ => None,
    };
    Ok(Loaded {
        from,
        to,
        change,
        title,
    })
}

/// A path of the UI to go back to after a form, never another site.
pub(super) fn safe_back(back: &str, default: &str) -> String {
    if back.starts_with('/') && !back.starts_with("//") && !back.starts_with("/\\") {
        back.to_string()
    } else {
        default.to_string()
    }
}

/// A recorded change, to review.
pub(super) async fn page(
    State(ui): State<Ui>,
    ctx: Ctx,
    Path(seq): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let row = ui
        .node
        .store
        .change(seq)?
        .ok_or_else(|| not_found("This change"))?;
    let l = load(&ui.node, &row.from_id, &row.to_id)?;
    // Newest first.
    let all = ui.node.store.changes(Some(&row.url), 10_000)?;
    let at = all.iter().position(|c| c.seq == seq);
    let newer = at.and_then(|i| i.checked_sub(1)).and_then(|i| all.get(i));
    let older = at.and_then(|i| all.get(i + 1));
    let here = format!("/change/{seq}");
    let mode = q.get("view").map_or("", String::as_str);
    let body = html! {
        (heading(&row.url, &l))
        div.card {
            div.card-head {
                div.tags {
                    (view::change_badge(row.silent, &row.summary))
                    (view::line_counts(row.added, row.removed))
                }
                div.sub { "Found " (view::when(row.detected_at)) }
            }
            (explain(&row.summary, row.silent, &l))
            div.actions {
                @match row.reviewed_at {
                    Some(t) => { span.muted.small { "✓ Reviewed " (view::when(t)) } }
                    None => {
                        (view::button(&ui, "/review", &[("seq", &seq.to_string()), ("back", &here)],
                            "Mark as reviewed", "primary small", None))
                    }
                }
                @if let Some(c) = newer { a.btn.ghost.small href={ "/change/" (c.seq) } { "← Newer change" } }
                @if let Some(c) = older { a.btn.ghost.small href={ "/change/" (c.seq) } { "Older change →" } }
            }
        }
        (body(&here, mode, &l))
    };
    let title = l.title.clone().unwrap_or_else(|| "Change".into());
    Ok(view::page(&ui, &ctx, Nav::Overview, &title, body))
}

/// Any two captures. A recorded change opens as one.
pub(super) async fn diff(
    State(ui): State<Ui>,
    ctx: Ctx,
    Query(q): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let id = |k: &str| -> WebResult<Digest> {
        q.get(k)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| not_found("A capture"))
    };
    let (from, to) = (id("from")?, id("to")?);
    let l = load(&ui.node, &from, &to)?;
    let url = l.to.signed.attestation.url.clone();
    if let Some(c) = ui
        .node
        .store
        .changes(Some(&url), 10_000)?
        .into_iter()
        .find(|c| c.from_id == from && c.to_id == to)
    {
        return Ok(redirect(&format!("/change/{}", c.seq)));
    }
    let here = format!("/diff?from={from}&to={to}");
    let mode = q.get("view").map_or("", String::as_str);
    let summary = l
        .change
        .as_ref()
        .map(|c| crate::diff_summary(&l.from.signed.attestation, &l.to.signed.attestation, c));
    let body = html! {
        (heading(&url, &l))
        div.card {
            @match (&l.change, &summary) {
                (Some(c), Some(s)) => {
                    div.tags { (view::change_badge(c.is_silent(), s)) (view::line_counts(c.added as u64, c.removed as u64)) }
                    (explain(s, c.is_silent(), &l))
                }
                _ => { p { "These two captures read the same." } }
            }
        }
        (body(&here, mode, &l))
    };
    Ok(view::page(&ui, &ctx, Nav::Pages, "Compare captures", body))
}

fn heading(url: &str, l: &Loaded) -> Markup {
    html! {
        nav.crumbs { a href=(view::page_link(url)) { "← " (view::address(url)) } }
        div.head {
            h1 { @if let Some(t) = &l.title { (t) } @else { (view::address(url)) } }
            a.btn.ghost.small href=(url) rel="noreferrer noopener" target="_blank" { "Open the live page ↗" }
        }
    }
}

/// What the change means, in words.
fn explain(summary: &str, silent: bool, l: &Loaded) -> Markup {
    let (a, b) = (&l.from.signed.attestation, &l.to.signed.attestation);
    let reasons = match l.change.as_ref().map(|c| &c.disclosure) {
        Some(Disclosure::Disclosed { reasons }) => reasons.clone(),
        _ => vec![],
    };
    html! {
        @if summary.starts_with("status ") {
            p {
                "The page answered with status " b { (a.status) } " before and " b { (b.status) } " now. "
                "That's usually an error page, a block or a redirect, not an edit."
            }
        } @else if silent {
            p {
                "The page changed, but it shows no new date and no update notice. "
                "Read the change closely: this is what Keepword exists to catch."
            }
        } @else {
            p { "The page says it was updated: " (reasons.join("; ")) "." }
        }
        @if !comparable(a, b) {
            p.notice.warn.small {
                "These two captures were made or read differently (a different capture method, "
                "or a Keepword upgrade in between), so part of the difference may come from that."
            }
        }
        dl.versions {
            dt { "Before" }
            dd { (view::absolute(a.fetched_at_ms)) " · " a href={ "/a/" (l.from.id) } { "evidence" } }
            dt { "After" }
            dd { (view::absolute(b.fetched_at_ms)) " · " a href={ "/a/" (l.to.id) } { "evidence" } }
        }
    }
}

/// The change itself, in the view asked for.
fn body(here: &str, mode: &str, l: &Loaded) -> Markup {
    let sep = if here.contains('?') { '&' } else { '?' };
    let tab = |m: &str, label: &str| {
        let href = if m.is_empty() {
            here.to_string()
        } else {
            format!("{here}{sep}view={m}")
        };
        html! { a href=(href) aria-current=[(mode == m).then_some("page")] { (label) } }
    };
    html! {
        section {
            div.section-head {
                h2 { "What changed" }
                nav.tabs {
                    (tab("", "Changes")) (tab("whole", "Whole page")) (tab("lines", "Technical"))
                }
            }
            @match &l.change {
                None => {
                    p.muted {
                        "This node keeps no text for one of these captures (see "
                        a href="/settings#content" { "what to keep" } "), so it can't show what changed."
                    }
                }
                Some(c) => {
                    @if mode == "lines" {
                        p.muted.small { "The page as Keepword reads and signs it, one block per line." }
                        (view::technical(c))
                    } @else {
                        (view::reading(c, mode == "whole"))
                    }
                }
            }
        }
    }
}

/// Changes of a page, for its own page: newest first.
pub(super) fn history(ui: &Ui, url: &str, rows: &[ChangeRow]) -> Markup {
    let back = view::page_link(url);
    html! {
        @if rows.is_empty() {
            p.muted { "No changes yet." }
        } @else {
            ul.list {
                @for c in rows {
                    li.row {
                        a.grow href={ "/change/" (c.seq) } {
                            (view::change_badge(c.silent, &c.summary)) " "
                            (view::line_counts(c.added, c.removed))
                        }
                        span.muted.small { (view::when(c.detected_at)) }
                        @if c.reviewed_at.is_none() {
                            (view::button(ui, "/review", &[("seq", &c.seq.to_string()), ("back", &back)],
                                "Mark as reviewed", "ghost small", None))
                        } @else {
                            span.muted.small { "✓ reviewed" }
                        }
                    }
                }
            }
        }
    }
}
