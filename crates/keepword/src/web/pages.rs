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

/// How long a request to the network lasts when added with pages.
pub(super) const REQUEST_DAYS: u64 = 7;

/// The form that adds pages: several at once, one per line.
pub(super) fn add_form(ui: &Ui, open: bool) -> Markup {
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
            label.check {
                input type="checkbox" name="network";
                " Also ask the network to watch them for " (REQUEST_DAYS) " days, so independent witnesses check them too"
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
    let search = q.get("q").map_or("", |s| s.trim()).to_lowercase();
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
                input type="search" name="q" value=(search) placeholder="Search addresses" data-filter=".list > li";
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
    let mut asked = 0;
    let mut failed = Vec::new();
    for u in &urls {
        ui.node.store.watch_add(u, every, render, now)?;
        if form.on("network") {
            let ms = (REQUEST_DAYS * 86_400_000) as i64;
            match ui.node.replace_request(u, every, ms, render) {
                Ok(_) => asked += 1,
                Err(e) => failed.push(format!("{u}: {e:#}")),
            }
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
    let hist = node.store.history(&url)?;
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
    let latest = hist.last();
    let title = match latest {
        Some(r) => node
            .normalized_text(&r.signed.attestation)?
            .as_deref()
            .and_then(view::title_of),
        None => None,
    };
    let changes = node.store.changes(Some(&url), 500)?;
    let to_review = changes.iter().filter(|c| c.reviewed_at.is_none()).count();
    let all = q.contains_key("all");
    let here = view::page_link(&url);
    let u = url.as_str();
    let body = html! {
        nav.crumbs { a href="/pages" { "← Pages" } }
        div.head {
            div {
                h1 { @if let Some(t) = &title { (t) } @else { (view::address(u)) } }
                @if title.is_some() { div.sub { (view::address(u)) } }
            }
            a.btn.ghost.small href=(u) rel="noreferrer noopener" target="_blank" { "Open the live page ↗" }
        }
        div.card {
            (status(&ui, u, watch.as_ref(), latest.map(|r| r.signed.attestation.fetched_at_ms)))
        }
        section {
            div.section-head {
                h2 { "Changes" }
                @if to_review > 1 {
                    (view::button(&ui, "/review", &[("url", u), ("back", &here)],
                        "Mark all as reviewed", "ghost small", None))
                }
            }
            (super::change::history(&ui, u, &changes))
        }
        section {
            h2 { "What other witnesses saw" }
            (verdict_view(&verdict))
            details.card.inline-card {
                summary { "Ask the network to watch this page" }
                (super::network::request_form(&ui, Some(u), watch.as_ref().map_or(3_600, |w| w.every_secs)))
            }
        }
        section {
            div.section-head {
                h2 { "Evidence" }
                span.muted.small { (hist.len()) " " (if hist.len() == 1 { "capture" } else { "captures" }) }
            }
            p.muted.small {
                "Each capture is signed by this node and logged where anyone can check it. "
                "Open one to download its evidence bundle."
            }
            ul.list {
                @for r in hist.iter().rev().take(if all { usize::MAX } else { 20 }) {
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
            @if hist.len() > 20 && !all {
                a.btn.ghost.small href={ (here) "&all=1" } { "Show all " (hist.len()) }
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
                        " · next check " (view::when(next.max(now_ms())))
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
    let msg = if ui.node.store.watch_remove(&url)? {
        "Stopped watching. Its records stay."
    } else {
        "It wasn't watched."
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
    fn pasted_addresses() {
        let (ok, bad) = parse_urls(
            "https://a.example/terms\n  b.example/privacy#top \nhttps://a.example/terms\nnot a url\nftp://c.example/",
        );
        assert_eq!(ok, ["https://a.example/terms", "https://b.example/privacy"]);
        assert_eq!(bad, ["not", "a", "url", "ftp://c.example/"]);
    }
}
