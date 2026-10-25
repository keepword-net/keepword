//! The network: how this node takes part, the pages it asked others to
//! watch, and the witnesses it knows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::response::Response;
use keepword_core::{now_ms, target};
use keepword_store::net::Peer;
use maud::{Markup, html};

use super::auth::{Post, flash};
use super::view::{self, Ctx, Nav};
use super::{Ui, WebResult, bad_request};
use crate::federation::PeerState;

/// How a witness has answered this node's syncs.
fn state_badge(p: &Peer, now: i64) -> Markup {
    let s = PeerState::of(p, now);
    let (class, why) = match s {
        PeerState::InSync => ("badge ok", "Its last sync worked, within the hour."),
        PeerState::Quiet => (
            "badge",
            "Its last sync worked, more than an hour ago. Each round syncs a few witnesses: \
             this node hasn't synced with it since, and doesn't know how it is doing.",
        ),
        PeerState::Failing => ("badge status", p.last_error.as_deref().unwrap_or_default()),
        PeerState::Never => ("badge", "This node hasn't synced with it yet."),
        PeerState::PushOnly => (
            "badge",
            "It has no address to sync with; it sends to this node.",
        ),
    };
    html! { span class=(class) title=(why) { (s.label()) } }
}

/// How long the network is asked to watch a page by default, and for as
/// long as a page watched here is watched (renewed).
const REQUEST_DAYS: u64 = (crate::federation::LEASE_MS / 86_400_000) as u64;

/// The form that asks the network to watch a page.
pub(super) fn request_form(ui: &Ui, url: Option<&str>, every: u64) -> Markup {
    if !super::joined(&ui.node) {
        return html! {
            p.muted {
                "This node works alone, so nobody else would hear the request. "
                a href="/settings#network" { "Join the network" } " first."
            }
        };
    }
    // A request for a page this node watches is asked again while it does
    // (`Node::renew_requests`): how long to ask for would mean nothing.
    let watched_here = url.is_some_and(|u| {
        ui.node
            .store
            .watches()
            .is_ok_and(|ws| ws.iter().any(|w| w.url == u && w.request_id.is_none()))
    });
    html! {
        form.stack method="post" action="/network/request" {
            (view::csrf(ui))
            @match url {
                Some(u) => { input type="hidden" name="u" value=(u); }
                None => {
                    label { "Page address"
                        input type="url" name="u" required placeholder="https://example.com/terms";
                    }
                }
            }
            div.row-fields {
                label { "Check" (view::interval_select("every", every)) }
                @if watched_here {
                    input type="hidden" name="days" value=(REQUEST_DAYS);
                } @else {
                    label { "For"
                        select name="days" {
                            @for d in [1u64, 3, 7, 14, 30] {
                                option value=(d) selected[d == REQUEST_DAYS] { (d) (if d == 1 { " day" } else { " days" }) }
                            }
                        }
                    }
                }
            }
            p.muted.small {
                "Witnesses drawn at random, in different countries and networks, capture it too, "
                "so nobody has to take this node's word for what the page said. "
                "Its address goes to every witness. "
                @if watched_here {
                    "It's asked again for as long as you watch the page; withdraw it on the Network page to stop."
                } @else {
                    "For a page this node watches, it's asked again while you watch it."
                }
            }
            div { button.btn.primary.small type="submit" { "Ask the network" } }
        }
    }
}

pub(super) async fn page(State(ui): State<Ui>, ctx: Ctx) -> WebResult<Response> {
    let node = &ui.node;
    let now = now_ms();
    let net = &node.config.network;
    let me = node.location_of(&node.key.public(), now)?;
    let peers = node.store.peers()?;
    let scores = node.store.reputation_scores(now)?;
    let bad = node.store.equivocating_logs()?;
    let mine = node.store.requests_by(&node.key.public(), now)?;
    let others = node
        .store
        .requests_active(now)?
        .iter()
        .filter(|r| r.body.requester != node.key.public())
        .count();
    let joined = super::joined(node);
    let watches = node.store.watches()?;
    let for_network = watches.iter().filter(|w| w.request_id.is_some()).count();
    // Pages watched here and asked about: this node keeps asking.
    let watched: std::collections::HashSet<&str> = watches
        .iter()
        .filter(|w| w.request_id.is_none() && w.network)
        .map(|w| w.url.as_str())
        .collect();
    // What `keepword net status` warns about, but that federation is off:
    // "Reachable" says so, with what to do.
    let warnings: Vec<String> = crate::status_warnings(node, me.is_some())?
        .into_iter()
        .filter(|w| !w.starts_with("federation is off"))
        .collect();
    let ok = peers
        .iter()
        .filter(|p| PeerState::of(p, now) == PeerState::InSync)
        .count();
    let last_sync = peers.iter().filter_map(|p| p.last_ok).max();
    let anchors = node.store.anchors()?;
    let relayed_by = crate::relay::Saved::load(&node.dir).map(|s| s.relay);

    let body = html! {
        div.head { h1 { "Network" } }
        div.card {
            div.card-head {
                h2 { "This node" }
                @if joined { (view::button(&ui, "/network/sync", &[], "Sync now", "small", None)) }
            }
            dl.facts {
                dt { "Reachable" }
                dd {
                    @match (&net.endpoint, &net.relay) {
                        (Some(e), _) => { "at " span.mono { (e) } }
                        (None, Some(_)) => {
                            "at home, through a relay"
                            @if let Some(r) = &relayed_by { ": " span.mono { (r) } } @else { " (looking for one)" }
                        }
                        (None, None) if !joined => {
                            "no: it works alone. " a href="/settings#network" { "Join the network" }
                        }
                        (None, None) => {
                            "no: it fetches from peers, but nobody can reach it. "
                            a href="/settings#network" { "Change" }
                        }
                    }
                }
                dt { "Location" }
                dd {
                    @match &me {
                        Some(l) => { (l.to_string()) }
                        None => { "not confirmed yet" span.muted.small { " (peers confirm it after a few syncs)" } }
                    }
                }
                dt { "Witnesses" }
                dd {
                    (peers.len()) " known, " (ok) " in sync"
                    @if let Some(t) = last_sync { " · last sync " (view::when(t)) }
                }
                dt { "Witness key" }
                dd.mono.break { (node.key.public()) }
            }
            @if !warnings.is_empty() {
                details.notice.warn {
                    summary { b { "Things to check (" (warnings.len()) ")" } }
                    ul { @for w in &warnings { li { (w) } } }
                }
            }
        }
        section #request {
            div.section-head {
                h2 { "Pages you asked the network to watch" }
                span.muted.small { (mine.len()) " of " (node.config.network.max_requests_per_requester) " in use" }
            }
            @if mine.is_empty() {
                p.muted { "None yet." }
            } @else {
                ul.list {
                    @for r in &mine {
                        li.row {
                            a.grow href=(view::page_link(&r.body.url)) {
                                (view::address(&r.body.url))
                                div.sub {
                                    (view::every(r.body.every_secs)) " · until " (view::absolute(r.body.expires_at_ms))
                                    @if watched.contains(r.body.url.as_str()) { " · asked again while you watch it" }
                                }
                            }
                            (view::button(&ui, "/network/withdraw", &[("u", &r.body.url)], "Withdraw", "ghost small",
                                Some("Withdraw this request? Witnesses stop after their next sync.")))
                        }
                    }
                }
            }
            details.card.inline-card open[mine.is_empty()] {
                summary { "Ask the network to watch a page" }
                (request_form(&ui, None, 3_600))
            }
        }
        section {
            h2 { "Watched for others" }
            p {
                "This node checks " a href="/pages?show=network" { (for_network) " " (if for_network == 1 { "page" } else { "pages" }) }
                " that other witnesses asked the network to watch. "
                span.muted { (others) " requests are active across the network." }
            }
        }
        section {
            div.section-head { h2 { "Witnesses" } }
            @if peers.is_empty() {
                p.muted { "This node doesn't know any witness yet." }
            } @else {
                div.table-wrap {
                    table {
                        thead { tr { th { "Witness" } th { "Where" } th { "Reputation" } th { "Last worked" } th { "State" } th {} } }
                        tbody {
                            @for p in &peers {
                                @let host = p.endpoint.as_deref().and_then(crate::relay::endpoint_host);
                                tr {
                                    td {
                                        span.mono { (p.key.short()) }
                                        @if let Some((h, relayed)) = &host {
                                            div.sub {
                                                @if *relayed { span title="Behind a router; this witness relays for it." { "through " (h) } }
                                                @else { (h) }
                                            }
                                        }
                                    }
                                    td { @match node.location_of(&p.key, now)? { Some(l) => { (l.to_string()) } None => { span.muted { "unknown" } } } }
                                    td { (format!("{:.1}", scores.get(&p.key).copied().unwrap_or(0.0))) }
                                    td { @match p.last_ok { Some(t) => { (view::when(t)) } None => { "never" } } }
                                    td {
                                        @if bad.contains(&p.key) {
                                            span.badge.silent title="Its log showed two different histories." { "equivocated" }
                                        } @else {
                                            (state_badge(p, now))
                                        }
                                    }
                                    td {
                                        (view::button(&ui, "/network/forget", &[("key", &p.key.to_hex())], "Forget", "ghost small",
                                            Some("Forget this witness? A live one comes back through gossip.")))
                                    }
                                }
                            }
                        }
                    }
                }
            }
            details.card.inline-card {
                summary { "Add a witness" }
                form.stack method="post" action="/network/peer" {
                    (view::csrf(&ui))
                    label { "Its address"
                        input type="url" name="url" required placeholder="https://witness.example.org";
                    }
                    div { button.btn.small type="submit" data-busy="Contacting…" { "Add witness" } }
                }
            }
        }
        @if !anchors.is_empty() {
            section {
                h2 { "Bitcoin anchors" }
                p.muted.small { "The log's state, timestamped in Bitcoin through OpenTimestamps: proof it existed by then." }
                ul.list {
                    @for a in anchors.iter().take(10) {
                        li.row {
                            span.grow { (a.size) " records" }
                            span.muted.small { (a.status) @if let Some(h) = a.height { " · block " (h) } }
                        }
                    }
                }
            }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Network, "Network", body))
}

pub(super) async fn request(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    if !super::joined(&ui.node) {
        return Ok(flash(
            "/network",
            "error",
            "This node works alone: join the network first.",
        ));
    }
    let url = target::canonical_url(form.get("u"))
        .map_err(|_| bad_request("That isn't a web address."))?
        .to_string();
    let every: u64 = form
        .get("every")
        .parse()
        .map_err(|_| bad_request("Choose how often."))?;
    let days: u64 = form
        .get("days")
        .parse()
        .map_err(|_| bad_request("Choose for how long."))?;
    let render = ui
        .node
        .store
        .watches()?
        .into_iter()
        .any(|w| w.url == url && w.render);
    let duration_ms = i64::try_from(days.saturating_mul(86_400_000)).unwrap_or(i64::MAX);
    Ok(
        match ui.node.replace_request(&url, every, duration_ms, render) {
            Ok((r, _)) => flash(
                "/network#request",
                "ok",
                format!(
                    "Asked the network to watch {}, {} until {}. Witnesses hear about it at the next sync.",
                    r.body.url,
                    view::every(every),
                    view::absolute(r.body.expires_at_ms)
                ),
            ),
            Err(e) => flash(
                "/network#request",
                "error",
                format!("The network didn't take it: {e:#}"),
            ),
        },
    )
}

pub(super) async fn withdraw(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let back = super::change::safe_back(form.get("back"), "/network#request");
    Ok(match ui.node.withdraw_requests(form.get("u"))? {
        // It ran out while the node was down: it isn't asked about again.
        0 => flash(&back, "ok", "It won't be asked about again."),
        _ => flash(
            &back,
            "ok",
            "Withdrawn. Witnesses stop after their next sync.",
        ),
    })
}

pub(super) async fn add_peer(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    Ok(match ui.node.add_peer(form.get("url")).await {
        Ok(d) => flash(
            "/network",
            "ok",
            format!("Added witness {}.", d.body.key.short()),
        ),
        Err(e) => flash("/network", "error", format!("Couldn't add it: {e:#}")),
    })
}

pub(super) async fn forget_peer(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let key = form.get("key");
    let peer = ui
        .node
        .store
        .peers()?
        .into_iter()
        .find(|p| p.key.to_hex() == key);
    Ok(match peer {
        Some(p) => {
            ui.node.store.peer_remove(&p.key)?;
            flash(
                "/network",
                "ok",
                format!("Forgot witness {}.", p.key.short()),
            )
        }
        None => flash("/network", "error", "No such witness."),
    })
}

/// A federation round started from the UI is running: pressing the button
/// again doesn't start another.
static SYNCING: AtomicBool = AtomicBool::new(false);

/// Run a federation round now. A slow one goes on in the background.
pub(super) async fn sync(State(ui): State<Ui>, _: Post) -> WebResult<Response> {
    if !super::joined(&ui.node) {
        return Ok(flash(
            "/network",
            "error",
            "This node works alone: join the network first.",
        ));
    }
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            SYNCING.store(false, Ordering::Release);
        }
    }
    if SYNCING.swap(true, Ordering::AcqRel) {
        return Ok(flash(
            "/network",
            "ok",
            "A sync is already running. Reload this page in a minute.",
        ));
    }
    let node = Arc::clone(&ui.node);
    let round = tokio::spawn(async move {
        let _done = Done;
        node.sync().await
    });
    Ok(
        match tokio::time::timeout(Duration::from_secs(20), round).await {
            Ok(Ok(Ok(r))) => {
                let mut msg = format!(
                    "Synced with {} of {} witnesses: {} new captures, {} messages in, {} out.",
                    r.synced, r.peers, r.new_attestations, r.gossip_in, r.gossip_out
                );
                if !r.errors.is_empty() {
                    msg.push_str(&format!(" {} failed.", r.errors.len()));
                }
                flash("/network", "ok", msg)
            }
            Ok(Ok(Err(e))) => flash("/network", "error", format!("Sync failed: {e:#}")),
            Ok(Err(e)) => flash("/network", "error", format!("Sync failed: {e}")),
            Err(_) => flash(
                "/network",
                "ok",
                "Still syncing in the background. Reload this page in a minute.",
            ),
        },
    )
}
