//! The overview: what needs a look, and whether the node is doing its job.

use axum::extract::State;
use axum::response::Response;
use keepword_core::now_ms;
use maud::{Markup, html};

use super::auth::{Post, flash};
use super::change::{load, safe_back};
use super::view::{self, Ctx, Nav};
use super::{Ui, WebResult};

/// Most changes the overview lists to review.
const TO_REVIEW: u32 = 25;

pub(super) async fn page(State(ui): State<Ui>, ctx: Ctx) -> WebResult<Response> {
    let node = &ui.node;
    let store = &node.store;
    let watches = store.watches()?;
    let own: Vec<_> = watches.iter().filter(|w| w.request_id.is_none()).collect();
    let failing = watches.iter().filter(|w| w.last_error.is_some()).count();
    let last_check = watches.iter().filter_map(|w| w.last_run).max();
    let review = store.changes_to_review(TO_REVIEW)?;
    let to_review = store.changes_to_review_count()?;
    let peers = store.peers()?;
    let net = &node.config.network;
    let joined = net.endpoint.is_some() || net.relay.is_some() || !peers.is_empty();
    let since = now_ms() - 7 * 86_400_000;
    let alerts: Vec<_> = store
        .alerts(50)?
        .into_iter()
        .filter(|a| a.body.issued_at_ms >= since)
        .collect();

    let mut cards = Vec::new();
    for c in &review {
        cards.push(review_card(&ui, c)?);
    }

    let body = html! {
        div.head {
            h1 { "Overview" }
        }
        @if !ui.opts.watching {
            div.notice.warn {
                b { "Automatic checks are off." }
                " Keepword only checks pages when you press “Check now”. Start it with "
                code { "keepword serve --watch" } " to check them on their schedule."
            }
        }
        @if own.is_empty() {
            (welcome(&ui, joined))
        } @else {
            div.stats {
                a.stat href="/pages" {
                    span.n { (own.len()) }
                    span.l { "pages watched" }
                }
                div.stat {
                    span.n { @if let Some(t) = last_check { (view::relative(t)) } @else { "never" } }
                    span.l { "last check" }
                }
                a.stat href="#review" {
                    span.n { (to_review) }
                    span.l { "changes to review" }
                }
                a.stat href="/network" {
                    @if joined {
                        span.n { (peers.len()) }
                        span.l { "witnesses known" }
                    } @else {
                        span.n { "Alone" }
                        span.l { "not in the network yet" }
                    }
                }
            }
            @if failing > 0 {
                div.notice.error {
                    b { (failing) " " (if failing == 1 { "page" } else { "pages" }) " failed their last check." }
                    " " a href="/pages?show=errors" { "See which" }
                }
            }
            @if !joined {
                div.notice {
                    b { "This node works alone." }
                    " Join the network so independent witnesses can confirm what pages said. "
                    a href="/settings#network" { "Set it up" }
                }
            }
        }
        section #review {
            div.section-head {
                h2 { "To review" }
                @if to_review > 1 {
                    (view::button(&ui, "/review", &[("all", "1")], "Mark all as reviewed", "ghost small",
                        Some("Mark every change as reviewed?")))
                }
            }
            @if review.is_empty() {
                div.empty {
                    p { "Nothing to review." }
                    p.muted { "When a page you watch changes, it shows up here with what changed." }
                }
            } @else {
                p.muted.small {
                    "A " b { "silent edit" } " changed a page without a new date or an update notice. "
                    "A " b { "disclosed edit" } " says it was updated."
                }
                div.cards { @for c in cards { (c) } }
                @if to_review > review.len() as u64 {
                    p.muted { "And " (to_review - review.len() as u64) " older ones; mark these as reviewed to see them." }
                }
            }
        }
        @if !alerts.is_empty() {
            section {
                div.section-head {
                    h2 { "Network alerts this week" }
                    a.btn.ghost.small href="/alerts" { "All alerts" }
                }
                ul.plain {
                    @for a in alerts.iter().take(5) {
                        li {
                            (super::alerts::kind_badge(a.body.kind)) " "
                            @if let Some(u) = &a.body.url { a href=(view::page_link(u)) { (view::address(u)) } " " }
                            span.muted.small { (view::when(a.body.issued_at_ms)) }
                        }
                    }
                }
            }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Overview, "Overview", body))
}

/// A change to review: where, what kind, the first changed lines.
fn review_card(ui: &Ui, c: &keepword_store::ChangeRow) -> WebResult<Markup> {
    let l = load(&ui.node, &c.from_id, &c.to_id)?;
    let href = format!("/change/{}", c.seq);
    Ok(html! {
        article.card.change {
            div.card-head {
                div {
                    a.title href=(href) {
                        @if let Some(t) = &l.title { (t) } @else { (view::address(&c.url)) }
                    }
                    div.sub {
                        @if l.title.is_some() { (view::address(&c.url)) " · " }
                        (view::when(c.detected_at))
                    }
                }
                div.tags {
                    (view::change_badge(c.silent, &c.summary))
                    (view::line_counts(c.added, c.removed))
                }
            }
            @match &l.change {
                Some(d) => { (view::preview(d, 3)) }
                None => { p.muted.small { "This node didn't keep the pages' text, so it can't show what changed." } }
            }
            div.actions {
                a.btn.small href=(href) { "See the change" }
                (view::button(ui, "/review", &[("seq", &c.seq.to_string())], "Mark as reviewed", "ghost small", None))
            }
        }
    })
}

/// A node watching nothing yet: what to do first.
fn welcome(ui: &Ui, joined: bool) -> Markup {
    html! {
        div.card.welcome {
            h2 { "Start by watching a page" }
            p {
                "Keepword checks the pages you give it, keeps a signed record of what each said, "
                "and tells you when one changes, especially when it changes without saying so."
            }
            (super::pages::add_form(ui, true))
        }
        ol.steps {
            li.done { "Keepword is running." }
            li { "Add the pages you care about: terms of service, privacy policies, prices, articles." }
            li class=[joined.then_some("done")] {
                "Join the network, so independent witnesses back up what yours saw. "
                @if !joined { a href="/settings#network" { "Set it up" } }
            }
        }
    }
}

/// Mark one change, a page's or all as reviewed.
pub(super) async fn review(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let seq = form.get("seq").parse::<i64>().ok();
    let url = Some(form.get("url")).filter(|u| !u.is_empty());
    if seq.is_none() && url.is_none() && !form.on("all") {
        return Err(super::bad_request("Nothing to mark as reviewed."));
    }
    let n = ui.node.store.mark_reviewed(seq, url, now_ms())?;
    let back = safe_back(form.get("back"), "/");
    Ok(flash(
        &back,
        "ok",
        match n {
            0 => "Already reviewed.".to_string(),
            1 => "Marked as reviewed.".to_string(),
            n => format!("Marked {n} changes as reviewed."),
        },
    ))
}
