//! What every page shares: the layout, times, page addresses, intervals,
//! and how normalized text and its changes read.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{Html, IntoResponse, Response};
use keepword_core::now_ms;
use keepword_normalize::diff::{self, Change, Op, OpTag, Words};
use maud::{DOCTYPE, Markup, html};

use super::Ui;
use super::auth::{Flash, UiAuth};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(super) const CSS: &str = include_str!("assets/app.css");
pub(super) const JS: &str = include_str!("assets/app.js");

/// What pages put in the assets' addresses, so a browser fetches them
/// again when they change.
static ASSETS_TAG: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let mut h = blake3::Hasher::new();
    h.update(CSS.as_bytes()).update(JS.as_bytes());
    h.finalize().to_hex()[..12].to_string()
});

pub(super) fn q(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The token every form posts.
pub(super) fn csrf(ui: &Ui) -> Markup {
    html! { input type="hidden" name="csrf" value=(ui.auth.csrf()); }
}

/// A form of one button, with hidden fields. `confirm` asks first (with
/// JavaScript).
pub(super) fn button(
    ui: &Ui,
    action: &str,
    fields: &[(&str, &str)],
    label: &str,
    class: &str,
    confirm: Option<&str>,
) -> Markup {
    html! {
        form.inline method="post" action=(action) data-confirm=[confirm] {
            (csrf(ui))
            @for (k, v) in fields { input type="hidden" name=(k) value=(v); }
            button type="submit" class={ "btn " (class) } { (label) }
        }
    }
}

/// What the layout needs from the request.
pub(super) struct Ctx {
    flash: Option<(String, String)>,
    to_review: u64,
}

impl FromRequestParts<Ui> for Ctx {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, ui: &Ui) -> Result<Self, Self::Rejection> {
        Ok(Ctx {
            flash: parts.extensions.get::<Flash>().and_then(|f| f.0.clone()),
            to_review: ui.node.store.changes_to_review_count().unwrap_or(0),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Nav {
    Overview,
    Pages,
    Network,
    Alerts,
    Settings,
}

fn head(title: &str) -> Markup {
    html! {
        head {
            meta charset="utf-8";
            meta name="viewport" content="width=device-width,initial-scale=1";
            meta name="color-scheme" content="light dark";
            title { (title) " · Keepword" }
            link rel="icon" href="/favicon.svg" type="image/svg+xml";
            link rel="stylesheet" href={ "/static/app.css?v=" (*ASSETS_TAG) };
            script src={ "/static/app.js?v=" (*ASSETS_TAG) } defer {}
        }
    }
}

fn flash_box(flash: &Option<(String, String)>) -> Markup {
    html! {
        @if let Some((kind, msg)) = flash {
            div class={ "flash " (kind) } role="status" { (msg) }
        }
    }
}

/// A page of the UI.
pub(super) fn page(ui: &Ui, ctx: &Ctx, nav: Nav, title: &str, body: Markup) -> Response {
    let item = |n: Nav, href: &str, label: &str, count: u64| {
        html! {
            a href=(href) aria-current=[(n == nav).then_some("page")] {
                (label)
                @if count > 0 { span.count { (count) } }
            }
        }
    };
    let markup = html! {
        (DOCTYPE)
        html lang="en" {
            (head(title))
            body {
                header.top {
                    div.bar {
                        a.brand href="/" {
                            img src="/favicon.svg" alt="" width="28" height="28";
                            span { "Keepword" }
                        }
                        nav.menu {
                            (item(Nav::Overview, "/", "Overview", ctx.to_review))
                            (item(Nav::Pages, "/pages", "Pages", 0))
                            (item(Nav::Network, "/network", "Network", 0))
                            (item(Nav::Alerts, "/alerts", "Alerts", 0))
                            (item(Nav::Settings, "/settings", "Settings", 0))
                        }
                        @if ui.auth.mode() == UiAuth::Password {
                            form.logout method="post" action="/logout" {
                                (csrf(ui))
                                button.btn.ghost.small type="submit" { "Log out" }
                            }
                        }
                    }
                }
                main.wrap {
                    (flash_box(&ctx.flash))
                    (body)
                }
                footer.wrap.muted.small {
                    "Keepword " (VERSION) " · witness " span.mono { (ui.node.key.public().short()) }
                    " · free software under the AGPL-3.0 · "
                    a href=(env!("CARGO_PKG_REPOSITORY")) rel="noreferrer" { "source code" }
                }
            }
        }
    };
    Html(markup.into_string()).into_response()
}

/// A page outside the UI's navigation: an error, a refusal.
pub(super) fn bare(title: &str, msg: &str) -> Html<String> {
    entry(
        title,
        &None,
        html! {
            p { (msg) }
            p { a href="/" { "Back to the overview" } }
        },
    )
}

/// A page with one card: logging in, setting up.
pub(super) fn entry(title: &str, flash: &Option<(String, String)>, body: Markup) -> Html<String> {
    Html(
        html! {
            (DOCTYPE)
            html lang="en" {
                (head(title))
                body.entry {
                    main.card.narrow {
                        div.brand {
                            img src="/favicon.svg" alt="" width="36" height="36";
                            span { "Keepword" }
                        }
                        h1 { (title) }
                        (flash_box(flash))
                        (body)
                    }
                }
            }
        }
        .into_string(),
    )
}

// -------------------------------------------------------------------- times

fn datetime(ms: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

/// A time for people: `22 Oct 2026, 14:43 UTC`.
pub(super) fn absolute(ms: i64) -> String {
    let f = time::macros::format_description!(
        "[day padding:none] [month repr:short] [year], [hour]:[minute] UTC"
    );
    datetime(ms).format(&f).unwrap_or_default()
}

/// How long ago, or how far ahead: `5 min ago`, `in 2 hours`.
pub(super) fn relative(ms: i64) -> String {
    let d = now_ms() - ms;
    let (past, s) = (d >= 0, d.unsigned_abs() / 1000);
    let span = match s {
        0..45 => return if past { "just now" } else { "in a moment" }.into(),
        45..90 => "a minute".to_string(),
        90..3_600 => format!("{} min", (s + 30) / 60),
        3_600..5_400 => "an hour".to_string(),
        5_400..86_400 => format!("{} hours", (s + 1_800) / 3_600),
        86_400..172_800 => "a day".to_string(),
        172_800..2_592_000 => format!("{} days", (s + 43_200) / 86_400),
        _ => {
            let f =
                time::macros::format_description!("[day padding:none] [month repr:short] [year]");
            return datetime(ms).format(&f).unwrap_or_default();
        }
    };
    if past {
        format!("{span} ago")
    } else {
        format!("in {span}")
    }
}

/// A time, relative, with the exact one on hover (in the reader's own
/// time zone, with JavaScript).
pub(super) fn when(ms: i64) -> Markup {
    let iso = keepword_core::format_ms(ms);
    html! { time datetime=(iso) title=(absolute(ms)) { (relative(ms)) } }
}

/// `every 6 hours`.
pub(super) fn every(secs: u64) -> String {
    let unit = |n: u64, one: &str, many: &str| match n {
        1 => format!("every {one}"),
        n => format!("every {n} {many}"),
    };
    match secs {
        s if s > 0 && s % 604_800 == 0 => unit(s / 604_800, "week", "weeks"),
        s if s > 0 && s % 86_400 == 0 => unit(s / 86_400, "day", "days"),
        s if s > 0 && s % 3_600 == 0 => unit(s / 3_600, "hour", "hours"),
        s if s > 0 && s % 60 == 0 => unit(s / 60, "minute", "minutes"),
        s => format!(
            "every {}",
            humantime::format_duration(std::time::Duration::from_secs(s))
        ),
    }
}

pub(super) const INTERVALS: &[(u64, &str)] = &[
    (900, "Every 15 minutes"),
    (1_800, "Every 30 minutes"),
    (3_600, "Every hour"),
    (10_800, "Every 3 hours"),
    (21_600, "Every 6 hours"),
    (43_200, "Every 12 hours"),
    (86_400, "Every day"),
    (604_800, "Every week"),
];

/// A choice of how often to check, with `secs` chosen.
pub(super) fn interval_select(name: &str, secs: u64) -> Markup {
    let listed = INTERVALS.iter().any(|(s, _)| *s == secs);
    html! {
        select name=(name) {
            @if !listed { option value=(secs) selected { (every(secs)) } }
            @for (s, label) in INTERVALS {
                option value=(s) selected[*s == secs] { (label) }
            }
        }
    }
}

// -------------------------------------------------------------------- pages

/// A page's address, its site first.
pub(super) fn address(url: &str) -> Markup {
    let parsed = url::Url::parse(url).ok();
    let host = parsed.as_ref().and_then(|u| {
        u.host_str()
            .map(|h| h.trim_start_matches("www.").to_string())
    });
    let rest = parsed.as_ref().map(|u| {
        let mut r = u.path().to_string();
        if let Some(q) = u.query() {
            r.push('?');
            r.push_str(q);
        }
        r
    });
    html! {
        span.addr title=(url) {
            @match (host, rest) {
                (Some(h), Some(r)) => { b { (h) } @if r != "/" { span.path { (r) } } }
                _ => { (url) }
            }
        }
    }
}

pub(super) fn page_link(url: &str) -> String {
    format!("/page?u={}", q(url))
}

/// The title line of normalized text.
pub(super) fn title_of(text: &str) -> Option<String> {
    text.lines()
        .take(8)
        .find_map(|l| l.strip_prefix("title: "))
        .map(str::to_string)
}

/// What kind of change: a silent edit, a disclosed one, or the page
/// answering differently (an error page against the page).
pub(super) fn change_badge(silent: bool, summary: &str) -> Markup {
    if summary.starts_with("status ") {
        html! { span.badge.status { "Answered differently" } }
    } else if silent {
        html! { span.badge.silent title="The page changed without saying so: no new date, no notice." { "Silent edit" } }
    } else {
        html! { span.badge.disclosed title="The page says it was updated." { "Disclosed edit" } }
    }
}

/// `+3 −1`, the lines added and removed.
pub(super) fn line_counts(added: u64, removed: u64) -> Markup {
    html! {
        span.counts {
            @if added > 0 { span.add { "+" (added) } }
            @if removed > 0 { span.rem { "−" (removed) } }
        }
    }
}

// --------------------------------------------------------- normalized text

/// What a normalized line's prefix means, for the metadata lines.
fn meta_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "title" => "Title",
        "description" => "Description",
        "published" => "Published",
        "modified" => "Last modified",
        "canonical" => "Canonical address",
        _ => return None,
    })
}

fn marked(words: &Words, class: &str) -> Markup {
    html! {
        @for (changed, w) in words {
            @if *changed { mark class=(class) { (w) } } @else { (w) }
        }
    }
}

/// One normalized line as it reads on the page; `words` marks what changed
/// in its text.
fn block(line: &str, words: Option<(&Words, &str)>) -> Markup {
    if line.starts_with("keepword-norm/") {
        return html! { div.b.meta { span.k { "Read as" } span.v.mono { (line) } } };
    }
    let (kind, text) = line.split_once(": ").unwrap_or(("text", line));
    let body = |plain: Markup| match words {
        Some((w, class)) => marked(w, class),
        None => plain,
    };
    if let Some(label) = meta_label(kind) {
        return html! { div.b.meta { span.k { (label) } span.v { (body(html! { (text) })) } } };
    }
    match kind {
        "link" | "img" => {
            let (target, desc) = text.split_once(" | ").unwrap_or((text, ""));
            let what = if kind == "link" { "Link" } else { "Image" };
            let plain = html! {
                @if !desc.is_empty() { span.desc { (desc) } " " }
                @if kind == "link" && (target.starts_with("https://") || target.starts_with("http://")) {
                    a.target href=(target) rel="noreferrer noopener" target="_blank" { (target) }
                } @else {
                    span.target { (target) }
                }
            };
            html! { div class={ "b " (kind) } { span.k { (what) } " " (body(plain)) } }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "quote" | "pre" | "cell" | "dt" | "dd"
        | "caption" => html! { div class={ "b " (kind) } { (body(html! { (text) })) } },
        _ => html! { div.b.p { (body(html! { (text) })) } },
    }
}

/// A diff, cut into what to show.
enum Piece<'a> {
    Same(&'a str),
    /// Unchanged lines away from any change.
    Hidden(Vec<&'a str>),
    /// Lines removed and the lines that replaced them.
    Changed(Vec<&'a str>, Vec<&'a str>),
}

fn pieces(ops: &[Op], radius: usize) -> Vec<Piece<'_>> {
    let near = |i: usize| {
        ops[i.saturating_sub(radius)..(i + radius + 1).min(ops.len())]
            .iter()
            .any(|o| o.tag != OpTag::Equal)
    };
    let mut out = Vec::new();
    let mut hidden = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        if ops[i].tag == OpTag::Equal {
            if near(i) {
                if !hidden.is_empty() {
                    out.push(Piece::Hidden(std::mem::take(&mut hidden)));
                }
                out.push(Piece::Same(&ops[i].line));
            } else {
                hidden.push(ops[i].line.as_str());
            }
            i += 1;
            continue;
        }
        if !hidden.is_empty() {
            out.push(Piece::Hidden(std::mem::take(&mut hidden)));
        }
        let run = |from: usize, tag: OpTag| -> Vec<&str> {
            ops[from..]
                .iter()
                .take_while(|o| o.tag == tag)
                .map(|o| o.line.as_str())
                .collect()
        };
        let del = run(i, OpTag::Delete);
        let ins = run(i + del.len(), OpTag::Insert);
        i += del.len() + ins.len();
        out.push(Piece::Changed(del, ins));
    }
    if !hidden.is_empty() {
        out.push(Piece::Hidden(hidden));
    }
    out
}

/// The text after a line's prefix.
fn text_of(line: &str) -> &str {
    line.split_once(": ").map_or(line, |(_, t)| t)
}

/// Lines removed and added, a removed line next to what it became, with
/// the words that changed marked.
fn changed(del: &[&str], ins: &[&str]) -> Markup {
    html! {
        div.chg {
            @for k in 0..del.len().max(ins.len()) {
                @match (del.get(k), ins.get(k)) {
                    (Some(a), Some(b)) => {
                        @let (wa, wb) = diff::words(text_of(a), text_of(b));
                        div.del { (block(a, Some((&wa, "del")))) }
                        div.ins { (block(b, Some((&wb, "ins")))) }
                    }
                    (Some(a), None) => { div.del { (block(a, None)) } }
                    (None, Some(b)) => { div.ins { (block(b, None)) } }
                    (None, None) => {}
                }
            }
        }
    }
}

/// Most lines a diff shows before it stops; the technical view and the
/// evidence have the rest.
const MAX_SHOWN: usize = 5_000;

/// Unchanged lines folded away beyond this many are only counted.
const MAX_FOLDED: usize = 400;

/// The pieces that fit in `MAX_SHOWN` lines, the last one cut short if it
/// doesn't, and whether any were left out: a page rewritten whole can have
/// a million lines.
fn shown(ps: Vec<Piece<'_>>) -> (Vec<Piece<'_>>, bool) {
    let mut left = MAX_SHOWN;
    let mut out = Vec::new();
    for p in ps {
        let n = match &p {
            Piece::Same(_) => 1,
            Piece::Hidden(ls) if ls.len() <= MAX_FOLDED => ls.len(),
            Piece::Hidden(_) => 1,
            Piece::Changed(d, i) => d.len() + i.len(),
        };
        if n > left {
            if let Piece::Changed(mut d, mut i) = p {
                d.truncate(left / 2);
                i.truncate(left - d.len());
                out.push(Piece::Changed(d, i));
            }
            return (out, true);
        }
        left -= n;
        out.push(p);
    }
    (out, false)
}

const TOO_LONG: &str =
    "This change is too long to show in full here; the evidence bundles have all of it.";

/// A change as the page reads: what changed, a little around it, and the
/// rest folded away (all of it with `whole`).
pub(super) fn reading(c: &Change, whole: bool) -> Markup {
    let (ps, cut) = shown(pieces(&c.ops, if whole { usize::MAX / 2 } else { 2 }));
    html! {
        div.doc {
            @for p in &ps {
                @match p {
                    Piece::Same(l) => { (block(l, None)) }
                    Piece::Hidden(ls) => {
                        @if ls.len() <= MAX_FOLDED {
                            details.same {
                                summary { (ls.len()) " unchanged " (lines(ls.len())) }
                                @for l in ls { (block(l, None)) }
                            }
                        } @else {
                            div.same.folded { (ls.len()) " unchanged lines" }
                        }
                    }
                    Piece::Changed(d, i) => { (changed(d, i)) }
                }
            }
            @if cut { p.muted { (TOO_LONG) } }
        }
    }
}

fn lines(n: usize) -> &'static str {
    if n == 1 { "line" } else { "lines" }
}

/// The first few changed lines of a change, for a list.
pub(super) fn preview(c: &Change, max: usize) -> Markup {
    let ps = pieces(&c.ops, 0);
    let mut changes = ps.iter().filter_map(|p| match p {
        Piece::Changed(d, i) => Some((d, i)),
        _ => None,
    });
    let mut left = max;
    let mut more = 0;
    let mut parts = Vec::new();
    for (d, i) in changes.by_ref() {
        if left == 0 {
            more += d.len().max(i.len());
            continue;
        }
        let take = d.len().max(i.len()).min(left);
        left -= take;
        more += d.len().max(i.len()) - take;
        let d: Vec<&str> = d.iter().take(take).copied().collect();
        let i: Vec<&str> = i.iter().take(take).copied().collect();
        parts.push(changed(&d, &i));
    }
    html! {
        div.doc.preview {
            @for p in parts { (p) }
            @if more > 0 { div.more.muted.small { "and " (more) " more changed " (lines(more)) } }
        }
    }
}

/// The normalized lines as they are, `+` and `-`: what was hashed.
pub(super) fn technical(c: &Change) -> Markup {
    let (ps, cut) = shown(pieces(&c.ops, 3));
    html! {
        @if cut { p.muted { (TOO_LONG) } }
        div.raw.mono {
            @for p in &ps {
                @match p {
                    Piece::Same(l) => { div { "  " (l) } }
                    Piece::Hidden(ls) => { div.gap { "… " (ls.len()) " unchanged " (lines(ls.len())) " …" } }
                    Piece::Changed(d, i) => {
                        @for l in d { div.del { "- " (l) } }
                        @for l in i { div.ins { "+ " (l) } }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        let now = now_ms();
        assert_eq!(relative(now - 10_000), "just now");
        assert_eq!(relative(now - 5 * 60_000), "5 min ago");
        assert_eq!(relative(now - 3 * 3_600_000), "3 hours ago");
        assert_eq!(relative(now + 2 * 3_600_000), "in 2 hours");
        assert_eq!(relative(now - 3 * 86_400_000), "3 days ago");
    }

    #[test]
    fn intervals() {
        assert_eq!(every(3_600), "every hour");
        assert_eq!(every(21_600), "every 6 hours");
        assert_eq!(every(86_400), "every day");
        assert_eq!(every(90), "every 1m 30s");
    }

    #[test]
    fn page_text_is_escaped() {
        let c = diff::diff(
            "p: <b>old</b>\n",
            "p: <script>alert(1)</script>\nlink: javascript:alert(1) | x\n",
        )
        .unwrap();
        let html = reading(&c, false).into_string();
        assert!(
            !html.contains("<script>") && !html.contains("<b>"),
            "{html}"
        );
        assert!(html.contains("&lt;script&gt;"));
        // Only web addresses are links.
        assert!(!html.contains("href=\"javascript:"), "{html}");
    }

    #[test]
    fn a_page_rewritten_whole_shows_in_part() {
        let a: String = (0..100_000).map(|i| format!("p: a{i}\n")).collect();
        let b: String = (0..100_000).map(|i| format!("p: b{i}\n")).collect();
        let c = diff::diff(&a, &b).unwrap();
        for html in [reading(&c, false), reading(&c, true), technical(&c)] {
            let html = html.into_string();
            assert!(html.contains(TOO_LONG));
            assert!(html.len() < 2_000_000, "{} bytes", html.len());
        }
    }

    #[test]
    fn changed_words_are_marked() {
        let c = diff::diff(
            "h1: T\np: kept for 30 days\n",
            "h1: T\np: kept for 3 years\n",
        )
        .unwrap();
        let html = preview(&c, 3).into_string();
        assert!(html.contains("<mark class=\"del\">30</mark>"), "{html}");
        assert!(html.contains("<mark class=\"ins\">3</mark>"), "{html}");
    }
}
