//! The pages a node watches: the list, adding, and each page's own view.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Response;
use keepword_core::quorum::{Group, Verdict};
use keepword_core::{now_ms, target};
use keepword_store::{UrlSummary, Watch};
use maud::{Markup, html};

use super::auth::{Post, flash, redirect};
use super::view::{self, Ctx, Nav};
use super::{Ui, WebResult, bad_request, not_found};
use crate::consensus::VerdictView;

/// This node's network requests in use, and how many the network takes
/// from one node.
pub(super) fn requests_in_use(ui: &Ui) -> (u64, u64) {
    let mine = ui
        .node
        .store
        .requests_by(&ui.node.key.public(), now_ms())
        .map_or(0, |r| r.len() as u64);
    (mine, ui.node.config.network.max_requests_per_requester)
}

/// Query parameters that carry a secret. A page behind one isn't public,
/// and asking the network would hand the secret to every witness.
const SECRET_PARAMS: &[&str] = &[
    "token",
    "auth",
    "key",
    "apikey",
    "sig",
    "signature",
    "session",
    "sessionid",
    "sid",
    "secret",
    "password",
    "pass",
    "code",
    "ticket",
];

/// Whether an address looks like it holds a secret. Only its query is
/// looked at: a secret in a path can't be told from a page's name.
fn looks_private(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.query_pairs().any(|(k, _)| {
            let k = k.to_ascii_lowercase();
            SECRET_PARAMS
                .iter()
                .any(|s| k == *s || k.ends_with(&format!("_{s}")) || k.ends_with(&format!("-{s}")))
        })
    })
}

/// The form that adds pages: several at once, one per line.
pub(super) fn add_form(ui: &Ui, open: bool) -> Markup {
    let (in_use, cap) = requests_in_use(ui);
    html! {
        form.stack.add method="post" action="/pages/add" {
            (view::csrf(ui))
            label {
                "Page addresses, one per line"
                textarea name="urls" rows=(if open { 4 } else { 3 }) required
                    placeholder="https://example.com/privacy\nhttps://example.com/terms" {}
            }
            div.row-fields {
                label { "Check" (view::interval_select("every", 3_600)) }
                @if cfg!(feature = "render") {
                    label.check {
                        input type="checkbox" name="render";
                        " Load them in a browser (for pages built by JavaScript)"
                    }
                }
            }
            @if super::joined(&ui.node) {
                div {
                    label.check {
                        input type="checkbox" name="network" checked[in_use < cap] disabled[in_use >= cap];
                        " Also ask the network to watch them, so independent witnesses check them too"
                    }
                    p.muted.small.hint {
                        "Their addresses go to every witness: leave this off for pages that aren't public. "
                        "Asked again while you watch them. "
                        (in_use) " of " (cap) " network requests in use"
                        @if in_use >= cap { ", none left until one runs out or is withdrawn" } "."
                    }
                }
            }
            div { button.btn.primary type="submit" data-busy="Adding…" { "Add pages" } }
        }
    }
}

/// One page in the list: its watch, its captures, or both.
struct Entry<'a> {
    url: &'a str,
    watch: Option<&'a Watch>,
    summary: Option<&'a UrlSummary>,
}

impl Entry<'_> {
    fn to_review(&self) -> u64 {
        self.summary.map_or(0, |s| s.to_review)
    }

    fn failed(&self) -> bool {
        self.watch.is_some_and(|w| w.last_error.is_some())
    }

    fn for_network(&self) -> bool {
        self.watch.is_some_and(|w| w.request_id.is_some())
    }
}

pub(super) async fn list(
    State(ui): State<Ui>,
    ctx: Ctx,
    Query(q): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let watches = ui.node.store.watches()?;
    let summaries = ui.node.store.urls()?;
    let mut by_url: BTreeMap<&str, Entry<'_>> = BTreeMap::new();
    for w in &watches {
        by_url.entry(&w.url).or_insert(Entry {
            url: &w.url,
            watch: None,
            summary: None,
        });
        by_url.get_mut(w.url.as_str()).expect("inserted").watch = Some(w);
    }
    for s in &summaries {
        by_url
            .entry(&s.url)
            .or_insert(Entry {
                url: &s.url,
                watch: None,
                summary: None,
            })
            .summary = Some(s);
    }
    let all: Vec<Entry<'_>> = by_url.into_values().collect();
    let show = q.get("show").map_or("all", String::as_str);
    let typed = q.get("q").map_or("", |s| s.trim());
    let search = typed.to_lowercase();
    let filters: [(&str, &str, usize); 5] = [
        ("all", "All", all.len()),
        (
            "review",
            "To review",
            all.iter().filter(|e| e.to_review() > 0).count(),
        ),
        (
            "errors",
            "Failing",
            all.iter().filter(|e| e.failed()).count(),
        ),
        (
            "unwatched",
            "Not watched",
            all.iter().filter(|e| e.watch.is_none()).count(),
        ),
        (
            "network",
            "For the network",
            all.iter().filter(|e| e.for_network()).count(),
        ),
    ];
    let mut shown: Vec<&Entry<'_>> = all
        .iter()
        .filter(|e| match show {
            "review" => e.to_review() > 0,
            "errors" => e.failed(),
            "unwatched" => e.watch.is_none(),
            "network" => e.for_network(),
            _ => true,
        })
        .filter(|e| search.is_empty() || e.url.to_lowercase().contains(&search))
        .collect();
    // What needs a look first, then what changed last.
    shown.sort_by_key(|e| {
        (
            std::cmp::Reverse(e.to_review()),
            !e.failed(),
            std::cmp::Reverse(e.summary.and_then(|s| s.last_change)),
        )
    });
    let body = html! {
        div.head { h1 { "Pages" } }
        @if !ui.opts.watching {
            div.notice.warn {
                b { "Automatic checks are off." } " Start Keepword with " code { "--watch" } " to check pages on their schedule."
            }
        }
        details.card open[all.is_empty()] {
            summary { b { "Add pages" } }
            (add_form(&ui, all.is_empty()))
        }
        @if !all.is_empty() {
            form.filters method="get" action="/pages" {
                input type="search" name="q" value=(typed) placeholder="Search addresses" data-filter=".list > li";
                input type="hidden" name="show" value=(show);
                noscript { button.btn.small type="submit" { "Search" } }
            }
            nav.chips {
                @for (key, label, n) in filters {
                    @if n > 0 || key == "all" {
                        a href={ "/pages?show=" (key) } aria-current=[(show == key).then_some("page")] {
                            (label) " " span.count { (n) }
                        }
                    }
                }
            }
            @if shown.is_empty() {
                p.muted { "No page matches." }
            } @else {
                ul.list {
                    @for e in &shown { (entry_row(e)) }
                }
            }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Pages, "Pages", body))
}

fn entry_row(e: &Entry<'_>) -> Markup {
    html! {
        li.row data-search=(e.url.to_lowercase()) {
            a.grow href=(view::page_link(e.url)) {
                (view::address(e.url))
                div.sub {
                    @match e.watch {
                        Some(w) => {
                            (view::every(w.every_secs))
                            " · checked "
                            @match w.last_run.or(e.summary.map(|s| s.last_capture)) {
                                Some(t) => { (view::when(t)) }
                                None => { "not yet" }
                            }
                        }
                        None => { "not watched" }
                    }
                    @if let Some(t) = e.summary.and_then(|s| s.last_change) {
                        " · changed " (view::when(t))
                    }
                }
            }
            div.tags {
                @if e.to_review() > 0 { span.badge.silent { (e.to_review()) " to review" } }
                @if e.failed() { span.badge.status { "check failed" } }
                @if e.for_network() { span.badge { "for the network" } }
            }
        }
    }
}

/// The URLs in the form, each made canonical, and the lines that weren't
/// URLs.
fn parse_urls(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    for word in text.split_whitespace() {
        // People paste addresses without the scheme; then a word is one
        // only with a dot in its host.
        let explicit = word.contains("://");
        let candidate = if explicit {
            word.to_string()
        } else {
            format!("https://{word}")
        };
        match target::canonical_url(&candidate) {
            Ok(u) if explicit || u.host_str().is_some_and(|h| h.contains('.')) => {
                if !ok.contains(&u.to_string()) {
                    ok.push(u.to_string());
                }
            }
            _ => bad.push(word.to_string()),
        }
    }
    (ok, bad)
}

fn interval(form: &Post) -> WebResult<u64> {
    let secs: u64 = form
        .get("every")
        .parse()
        .map_err(|_| bad_request("Choose how often to check."))?;
    if secs < 60 {
        return Err(bad_request("Check at most once a minute."));
    }
    Ok(secs)
}

pub(super) async fn add(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let every = interval(&form)?;
    let render = form.on("render") && cfg!(feature = "render");
    let (urls, bad) = parse_urls(form.get("urls"));
    if urls.is_empty() {
        return Ok(flash(
            "/pages",
            "error",
            "That isn't a web address. Paste addresses like https://example.com/terms.",
        ));
    }
    let now = now_ms();
    let ask = form.on("network") && super::joined(&ui.node);
    let (in_use, cap) = requests_in_use(&ui);
    let mut room = cap.saturating_sub(in_use);
    let (mut asked, mut over) = (0, 0);
    let (mut private, mut failed) = (Vec::new(), Vec::new());
    for u in &urls {
        ui.node.store.watch_add(u, every, render, now)?;
        if !ask {
            continue;
        }
        if looks_private(u) {
            private.push(u.as_str());
            continue;
        }
        if room == 0 {
            over += 1;
            continue;
        }
        match ui
            .node
            .replace_request(u, every, crate::federation::LEASE_MS, render)
        {
            // Asking again for a page replaces the request: no more in use.
            Ok((_, replaced)) => {
                asked += 1;
                if replaced == 0 {
                    room -= 1;
                }
            }
            Err(e) => failed.push(format!("{u}: {e:#}")),
        }
    }
    let mut msg = match urls.len() {
        1 => format!("Watching {}, {}.", urls[0], view::every(every)),
        n => format!("Watching {n} pages, {}.", view::every(every)),
    };
    if ui.opts.watching {
        msg.push_str(" The first check runs within a minute.");
    }
    if asked > 0 {
        msg.push_str(" The network hears about them at the next sync.");
    }
    if !private.is_empty() {
        msg.push_str(&format!(
            " Not asked the network about what looks private (a secret in its address): {}.",
            private.join(", ")
        ));
    }
    if over > 0 {
        msg.push_str(&format!(
            " {over} more weren't asked: this node has {cap} network requests in use, \
             all the network takes from one node."
        ));
    }
    if !bad.is_empty() {
        msg.push_str(&format!(
            " Skipped what isn't an address: {}.",
            bad.join(", ")
        ));
    }
    if !failed.is_empty() {
        msg.push_str(&format!(
            " Couldn't ask the network: {}.",
            failed.join("; ")
        ));
    }
    let to = match urls.as_slice() {
        [one] => view::page_link(one),
        _ => "/pages".to_string(),
    };
    Ok(flash(&to, "ok", msg))
}

fn form_url(form: &Post) -> WebResult<String> {
    target::canonical_url(form.get("u"))
        .map(|u| u.to_string())
        .map_err(|_| bad_request("That isn't a web address."))
}

/// The old address of a page's view.
pub(super) async fn old_link(Query(q): Query<HashMap<String, String>>) -> Response {
    redirect(&view::page_link(q.get("u").map_or("", String::as_str)))
}

pub(super) async fn detail(
    State(ui): State<Ui>,
    ctx: Ctx,
    Query(q): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let raw = q.get("u").ok_or_else(|| not_found("This page"))?;
    let url = target::canonical_url(raw)
        .map_err(|_| bad_request("That isn't a web address."))?
        .to_string();
    let node = &ui.node;
    let watch = node.store.watches()?.into_iter().find(|w| w.url == url);
    // A page checked every quarter hour has 35 000 captures a year: only
    // the latest are shown, `keepword history` lists them all.
    let more = q.contains_key("more");
    let hist = node
        .store
        .history_latest(&url, if more { 500 } else { 20 })?;
    let captures = node.store.capture_count(&url)?;
    let verdict = node.verdict(&url)?;
    if hist.is_empty() && watch.is_none() && verdict.considered == 0 {
        return Err(not_found("This page"));
    }
    // Ask the witnesses assigned to it for their latest captures, for the
    // next look: the page doesn't wait for them.
    if !node.store.peers()?.is_empty() {
        let n = Arc::clone(node);
        let u = url.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(30), n.refresh_url(&u)).await;
        });
    }
    let latest = hist.first();
    let title = match latest {
        Some(r) => node
            .normalized_text(&r.signed.attestation)?
            .as_deref()
            .and_then(view::title_of),
        None => None,
    };
    // An active request for it: one can run out while the node is down.
    let asked = node
        .store
        .requests_by(&node.key.public(), now_ms())?
        .iter()
        .any(|r| r.body.url == url);
    let changes = node.store.changes(Some(&url), 500)?;
    let to_review = changes.iter().filter(|c| c.reviewed_at.is_none()).count();
    let newest = changes.iter().map(|c| c.seq).max().unwrap_or(0);
    let here = view::page_link(&url);
    let u = url.as_str();
    let side = html! {
        div.card {
            (status(&ui, u, watch.as_ref(), latest.map(|r| r.signed.attestation.fetched_at_ms)))
        }
        section {
            h2 { "What other witnesses saw" }
            (verdict_view(&verdict))
        }
        @if watch.as_ref().is_some_and(|w| w.network) {
            div.card {
                @if asked {
                    p { b { "The network watches this page too." } " This node asks again while it watches it." }
                } @else {
                    p { b { "This node asks the network to watch this page" } " again at its next sync, as its request ran out." }
                }
                div.actions {
                    (view::button(&ui, "/network/withdraw", &[("u", u), ("back", &here)], "Withdraw", "ghost small",
                        Some("Stop asking the network to watch this page? Witnesses stop after their next sync.")))
                }
            }
        } @else {
            details.card {
                summary { "Ask the network to watch this page" }
                (super::network::request_form(&ui, Some(u), watch.as_ref().map_or(3_600, |w| w.every_secs)))
            }
        }
    };
    let main = html! {
        section {
            div.section-head {
                h2 { "Changes" }
                @if to_review > 1 {
                    (view::button(&ui, "/review", &[("url", u), ("upto", &newest.to_string()), ("back", &here)],
                        "Mark all as reviewed", "ghost small", None))
                }
            }
            (super::change::history(&ui, u, &changes))
        }
        section {
            div.section-head {
                h2 { "Evidence" }
                span.muted.small { (captures) " " (if captures == 1 { "capture" } else { "captures" }) }
            }
            p.muted.small {
                "Each capture is signed by this node and logged where anyone can check it. "
                "Open one to download its evidence bundle."
            }
            ul.list {
                @for r in &hist {
                    @let a = &r.signed.attestation;
                    li.row {
                        a.grow href={ "/a/" (r.id) } {
                            (view::absolute(a.fetched_at_ms))
                            div.sub.mono { (r.id.short()) " · " (crate::method_name(a.method)) }
                        }
                        span class={ "badge" @if a.status >= 400 { " status" } } { "HTTP " (a.status) }
                    }
                }
            }
            @if captures > hist.len() as u64 {
                @if more {
                    p.muted.small { "The latest " (hist.len()) " of " (captures) ". " code { "keepword history" } " lists them all." }
                } @else {
                    a.btn.ghost.small href={ (here) "&more=1" } { "Show more" }
                }
            }
        }
        details.danger {
            summary { "Delete what this node kept of this page" }
            p {
                "For an erasure request: deletes the page's content (its text and raw copies) from this node. "
                "The signed records stay, so the log remains intact, unless you also forget them."
            }
            form.stack method="post" action="/page/purge" data-confirm="Delete what this node kept of this page? This can't be undone." {
                (view::csrf(&ui))
                input type="hidden" name="u" value=(u);
                label.check { input type="checkbox" name="forget"; " Also forget the records (the log keeps only their IDs)" }
                div { button.btn.danger type="submit" { "Delete content" } }
            }
        }
    };
    let body = html! {
        nav.crumbs { a href="/pages" { "← Pages" } }
        div.head {
            div {
                h1 { @if let Some(t) = &title { (t) } @else { (view::address(u)) } }
                @if title.is_some() { div.sub { (view::address(u)) } }
            }
            a.btn.ghost.small href=(u) rel="noreferrer noopener" target="_blank" { "Open the live page ↗" }
        }
        (view::columns(main, side, true))
    };
    Ok(view::page(
        &ui,
        &ctx,
        Nav::Pages,
        title.as_deref().unwrap_or(&url),
        body,
    ))
}

/// Whether and how a page is watched, and what to do about it.
fn status(ui: &Ui, u: &str, watch: Option<&Watch>, last: Option<i64>) -> Markup {
    let check = html! {
        form.inline method="post" action="/page/check" {
            (view::csrf(ui))
            input type="hidden" name="u" value=(u);
            button.btn.primary.small type="submit" data-busy="Checking…" { "Check now" }
        }
    };
    html! {
        @match watch {
            Some(w) if w.request_id.is_some() => {
                p {
                    "This node checks this page for the network, " (view::every(w.every_secs))
                    @if let Some(t) = w.expires_at { ", until " (view::absolute(t)) } "."
                }
                (last_check(w, last))
                div.actions {
                    (check)
                    (watch_form(ui, u, w.every_secs, "Watch it myself"))
                }
            }
            Some(w) => {
                p {
                    "Checked " (view::every(w.every_secs))
                    @if let Some(t) = w.last_run {
                        @let next = t + (w.every_secs as i64) * 1000;
                        @if next > now_ms() { " · next check " (view::when(next)) } @else { " · a check is due" }
                    }
                    "."
                }
                (last_check(w, last))
                div.actions {
                    (check)
                    (watch_form(ui, u, w.every_secs, "Save"))
                    (view::button(ui, "/page/unwatch", &[("u", u)], "Stop watching", "ghost small",
                        Some("Stop watching this page? Its records stay.")))
                }
            }
            None => {
                p {
                    "Not watched"
                    @if let Some(t) = last { "; last captured " (view::when(t)) } "."
                }
                div.actions {
                    (check)
                    (watch_form(ui, u, 3_600, "Start watching"))
                }
            }
        }
    }
}

fn last_check(w: &Watch, last: Option<i64>) -> Markup {
    html! {
        @match (&w.last_error, w.last_run.or(last)) {
            (Some(e), _) => { p.notice.error.small { b { "The last check failed: " } (e) } }
            (None, Some(t)) => { p.muted.small { "Last checked " (view::when(t)) "." } }
            (None, None) => { p.muted.small { "Not checked yet." } }
        }
    }
}

fn watch_form(ui: &Ui, u: &str, every: u64, label: &str) -> Markup {
    html! {
        form.inline method="post" action="/page/watch" {
            (view::csrf(ui))
            input type="hidden" name="u" value=(u);
            (view::interval_select("every", every))
            button.btn.small type="submit" { (label) }
        }
    }
}

pub(super) async fn check(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let url = form_url(&form)?;
    let back = view::page_link(&url);
    let watch = ui.node.store.watches()?.into_iter().find(|w| w.url == url);
    let render = watch.as_ref().is_some_and(|w| w.render);
    let out = ui.node.capture(&url, render).await;
    if watch.is_some() {
        let err = out.as_ref().err().map(|e| format!("{e:#}"));
        ui.node.store.watch_mark(&url, now_ms(), err.as_deref())?;
    }
    let o = match out {
        Ok(o) => o,
        Err(e) => return Ok(flash(&back, "error", format!("The check failed: {e:#}"))),
    };
    if let Some(c) = &o.change {
        let row = ui
            .node
            .store
            .changes(Some(&url), 5)?
            .into_iter()
            .find(|r| r.to_id == o.record.id);
        let to = row.map_or(back, |r| format!("/change/{}", r.seq));
        let kind = if c.silent { "error" } else { "ok" };
        return Ok(flash(
            &to,
            kind,
            format!("The page changed: {}.", c.summary),
        ));
    }
    let msg = match (&o.previous, o.compared) {
        (None, _) => "First capture of this page saved.",
        (Some(_), false) => {
            "Captured. Keepword reads pages differently since the last capture, so it starts comparing from this one."
        }
        (Some(_), true) => "Checked: no change since the last capture.",
    };
    Ok(flash(&back, "ok", msg))
}

pub(super) async fn watch(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let url = form_url(&form)?;
    let every = interval(&form)?;
    let render = ui
        .node
        .store
        .watches()?
        .into_iter()
        .find(|w| w.url == url)
        .is_some_and(|w| w.render);
    ui.node.store.watch_add(&url, every, render, now_ms())?;
    Ok(flash(
        &view::page_link(&url),
        "ok",
        format!("Watching it, {}.", view::every(every)),
    ))
}

pub(super) async fn unwatch(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let url = form_url(&form)?;
    let asked = ui
        .node
        .store
        .watches()?
        .iter()
        .any(|w| w.url == url && w.network);
    let msg = match ui.node.store.watch_remove(&url)? {
        // Its request isn't renewed any more, but runs its time out.
        true if asked => {
            "Stopped watching. Its records stay. The network watches it for up to a week \
             more, until its request runs out; withdraw it on the Network page to stop now."
        }
        true => "Stopped watching. Its records stay.",
        false => "It wasn't watched.",
    };
    Ok(flash(&view::page_link(&url), "ok", msg))
}

pub(super) async fn purge(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let url = form_url(&form)?;
    let forget = form.on("forget");
    let (blobs, rows) = ui.node.store.purge(&url, forget)?;
    let msg = if forget {
        format!("Deleted {blobs} stored files and forgot {rows} records. The log is unchanged.")
    } else {
        format!("Deleted {blobs} stored files. The signed records stay.")
    };
    let to = if forget && ui.node.store.watches()?.iter().all(|w| w.url != url) {
        "/pages".to_string()
    } else {
        view::page_link(&url)
    };
    Ok(flash(&to, "ok", msg))
}

/// What independent witnesses saw of a page, in words.
fn verdict_view(v: &VerdictView) -> Markup {
    let groups = |gs: &[Group]| {
        html! {
            table.groups {
                thead { tr { th { "Version" } th { "Witnesses" } th { "Networks" } } }
                tbody {
                    @for g in gs {
                        tr {
                            td.mono { (g.hash.short()) }
                            td { (g.witnesses.len()) }
                            td { (g.asns.iter().map(|a| format!("AS{a}")).collect::<Vec<_>>().join(", ")) }
                        }
                    }
                }
            }
        }
    };
    html! {
        div.card {
            @match &v.evaluation.verdict {
                Verdict::Agreed { group, dissenters } => {
                    p.good {
                        b { "They agree." } " " (group.witnesses.len()) " witnesses in "
                        (group.asns.len()) " independent networks saw the same version."
                    }
                    @if !dissenters.is_empty() {
                        p.muted.small { (dissenters.len()) " saw something else, too few to count." }
                    }
                }
                Verdict::Split { groups: gs } => {
                    p.bad {
                        b { "They saw different versions at the same time." }
                        " The site shows visitors different things. That's often harmless (language, "
                        "an A/B test, a rollout, bot blocking), so compare the versions before calling it cloaking."
                    }
                    (groups(gs))
                }
                Verdict::Disputed { groups: gs, pending } => {
                    p {
                        b { "They disagree. " }
                        @if *pending {
                            "Witnesses drawn at random from the same countries are checking again; only versions they reproduce will count."
                        } @else {
                            "Checking again didn't settle which version the page served."
                        }
                    }
                    (groups(gs))
                }
                Verdict::Insufficient { groups: gs } => {
                    p.muted {
                        "Not enough independent witnesses checked this page recently ("
                        (v.considered) " " (if v.considered == 1 { "capture" } else { "captures" })
                        "). Ask the network to watch it to hear from them."
                    }
                    @if !gs.is_empty() { (groups(gs)) }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_looking_addresses() {
        for private in [
            "https://a.example/doc?token=x",
            "https://a.example/doc?api_key=x",
            "https://a.example/s?Session=1&lang=de",
            "https://a.example/invite?promo-code=x",
        ] {
            assert!(looks_private(private), "{private}");
        }
        for public in [
            "https://a.example/terms",
            "https://a.example/terms?lang=de&v=2",
            "https://a.example/keys-and-tokens",
            "https://a.example/p?monkey=1",
        ] {
            assert!(!looks_private(public), "{public}");
        }
    }

    #[test]
    fn pasted_addresses() {
        let (ok, bad) = parse_urls(
            "https://a.example/terms\n  b.example/privacy#top \nhttps://a.example/terms\nnot a url\nftp://c.example/",
        );
        assert_eq!(ok, ["https://a.example/terms", "https://b.example/privacy"]);
        assert_eq!(bad, ["not", "a", "url", "ftp://c.example/"]);
    }
}
