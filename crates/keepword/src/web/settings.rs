//! Settings: the ones most nodes change, as a form; the whole of
//! `keepword.toml` for the rest; the password; and a restart to apply
//! them, since the node reads its settings when it starts.

use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use keepword_core::net::AlertKind;
use maud::{Markup, html};

use super::auth::{
    Post, UiAuth, check_new_password, check_password, flash, relogin, store_password,
};
use super::view::{self, Ctx, Nav};
use super::{RESTART, Ui, WebResult};
use crate::config::{
    CONFIG_FILE, Config, NotifyConfig, NotifyFrom, NotifyTarget, Retain, TargetKind,
};
use crate::notify;

/// A setting the container sets from this variable on every start, so a
/// change here wouldn't last.
fn from_env(name: &'static str) -> Option<&'static str> {
    let v = std::env::var(name).ok()?;
    // The entrypoint only acts on KEEPWORD_SEEDS=0.
    let set = if name == "KEEPWORD_SEEDS" {
        v == "0"
    } else {
        !v.is_empty()
    };
    set.then_some(name)
}

fn locked(var: &str) -> Markup {
    html! {
        p.notice.small {
            "Set by " code { (var) } " in the container's settings; change it there."
        }
    }
}

pub(super) async fn page(State(ui): State<Ui>, ctx: Ctx) -> WebResult<Response> {
    let node = &ui.node;
    let file = std::fs::read_to_string(node.dir.join(CONFIG_FILE)).unwrap_or_default();
    // Changed by hand, the file may not be valid: the node runs on, but
    // wouldn't start again. Only the file can be edited then.
    let saved = match Config::load(&node.dir) {
        Ok(c) => c,
        Err(e) => {
            let body = html! {
                div.measure {
                    div.head { h1 { "Settings" } }
                    div.notice.error {
                        b { "keepword.toml isn't valid: " } (format!("{e:#}")) ". "
                        "Keepword runs on with the settings it started with, but won't start again \
                         until the file is fixed. Fix it here."
                    }
                    (file_editor(&ui, &file, true))
                }
            };
            return Ok(view::page(&ui, &ctx, Nav::Settings, "Settings", body));
        }
    };
    let pending = toml::to_string(&saved)? != toml::to_string(&node.config)?;
    let located = node.location_of(&node.key.public(), keepword_core::now_ms())?;
    let mode_env = from_env("KEEPWORD_ENDPOINT").or_else(|| from_env("KEEPWORD_RELAY"));
    let net = &saved.network;
    let mode = match (&net.endpoint, &net.relay) {
        (Some(_), _) => "endpoint",
        (None, Some(_)) => "relay",
        (None, None) if net.peers.is_empty() => "alone",
        (None, None) => "",
    };
    let radio = |value: &str, label: &str, help: &str| {
        html! {
            label.choice {
                input type="radio" name="mode" value=(value) checked[mode == value] disabled[mode_env.is_some()];
                span { b { (label) } br; span.muted.small { (help) } }
            }
        }
    };
    let retain = |value: Retain, key: &str, label: &str, help: &str| {
        html! {
            label.choice {
                input type="radio" name="retain" value=(key) checked[saved.content.retain == value];
                span { b { (label) } br; span.muted.small { (help) } }
            }
        }
    };
    let body = html! {
        div.measure {
            div.head { h1 { "Settings" } }
            @if pending {
                div.notice.warn #restart {
                    p { b { "Saved settings take effect after a restart." } }
                    (restart_control(&ui, true))
                }
            }
            form.stack.settings method="post" action="/settings" {
                (view::csrf(&ui))
                section.card #location {
                    h2 { "Where this node is" }
                    p.muted.small {
                        "The network spreads witnesses across countries and internet providers, so that "
                        "no single one can fake what a page said. Peers confirm where a node is from "
                        "where it connects; this is what it says about itself."
                    }
                    @if let Some(l) = &located {
                        p.small { "Peers see this node in " b { (l.to_string()) } "." }
                    }
                    div.row-fields {
                        label { "Country code"
                            input type="text" name="country" value=[saved.vantage.country.as_deref()]
                                maxlength="2" placeholder="DE" autocomplete="off" pattern="[A-Za-z]{2}";
                        }
                        label { "Network number (ASN)"
                            input type="text" name="asn" inputmode="numeric" value=[saved.vantage.asn]
                                placeholder="3320" autocomplete="off";
                        }
                    }
                    p.muted.small { "The ASN is your internet provider's number; sites like bgp.he.net show it for your address. Leave it empty if unsure." }
                }
                section.card #network {
                    h2 { "Joining the network" }
                    @if let Some(var) = mode_env { (locked(var)) }
                    @if mode.is_empty() {
                        p.small { "Now: this node contacts the witnesses listed below without being reachable itself." }
                    }
                    div.choices {
                        (radio("alone", "Work alone", "Keep records on this node only. Nobody else confirms what it saw."))
                        (radio("relay", "At home, behind a router", "Join through another witness that relays for this one. Nothing to set up on the router."))
                        (radio("endpoint", "On a server with a public address", "Peers connect to this node's API at the address below (put HTTPS in front of it)."))
                    }
                    label { "Public address of this node's API"
                        input type="url" name="endpoint" value=[net.endpoint.as_deref()]
                            placeholder="https://witness.example.org" disabled[mode_env.is_some()];
                    }
                    details {
                        summary { "More network settings" }
                        @if let Some(var) = from_env("KEEPWORD_SEEDS") { (locked(var)) }
                        label.check {
                            input type="checkbox" name="seeds" checked[net.seeds] disabled[from_env("KEEPWORD_SEEDS").is_some()];
                            " Join through Keepword's public seed witnesses"
                        }
                        @if let Some(var) = from_env("KEEPWORD_PEERS") { (locked(var)) }
                        label { "Other witnesses to join through, one address per line"
                            textarea name="peers" rows="3" disabled[from_env("KEEPWORD_PEERS").is_some()] {
                                (net.peers.join("\n"))
                            }
                        }
                    }
                }
                section.card #content {
                    h2 { "What to keep" }
                    p.muted.small { "The signed records are always kept; they hold no page content. This is about the pages' content." }
                    div.choices {
                        (retain(Retain::Normalized, "normalized", "The text (recommended)", "Enough to show what changed. Small."))
                        (retain(Retain::Full, "full", "The text and the raw pages", "Full evidence anyone can re-check byte for byte. Takes the most space."))
                        (retain(Retain::None, "none", "Nothing", "Changes are detected but can't be shown. For pages you may not store."))
                    }
                }
                section.card #capture {
                    h2 { "Capturing" }
                    div.row-fields {
                        label { "Give up on a page after (seconds)"
                            input type="number" name="timeout" min="5" max="300" value=(saved.capture.timeout_secs);
                        }
                    }
                    label { "User agent sent to sites (empty for Keepword's own)"
                        input type="text" name="user_agent" value=[saved.capture.user_agent.as_deref()] autocomplete="off";
                    }
                }
                div.actions { button.btn.primary type="submit" { "Save settings" } }
            }
            (notify_section(&ui, &saved))
            section.card #password {
                h2 { "Password" }
                (password_section(&ui))
            }
            (file_editor(&ui, &file, false))
            section.card {
                h2 { "About this node" }
                dl.facts {
                    dt { "Version" } dd { "Keepword " (env!("CARGO_PKG_VERSION")) }
                    dt { "Data folder" } dd.mono.break { (node.dir.display()) }
                    dt { "Witness key" } dd.mono.break { (node.key.public()) }
                    dt { "Automatic checks" } dd { @if ui.opts.watching { "on" } @else { "off (start with --watch)" } }
                    dt { "Restart" } dd { (restart_control(&ui, false)) }
                }
            }
        }
    };
    Ok(view::page(&ui, &ctx, Nav::Settings, "Settings", body))
}

/// A target's address, mostly hidden: whoever has it can post there.
fn masked(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else {
        return "…".into();
    };
    let path: String = u.path().chars().take(4).collect();
    format!("{}{path}…", u.host_str().unwrap_or_default())
}

fn notify_section(ui: &Ui, saved: &Config) -> Markup {
    let cfg = &saved.notify;
    let state = notify::State::load(&ui.node.dir);
    let running = &ui.node.config.notify.targets;
    let kind_box = |kind: AlertKind, name: &str, label: &str| {
        html! {
            label.check { input type="checkbox" name=(name) checked[cfg.kinds.contains(&kind)]; (label) }
        }
    };
    let from = |value: NotifyFrom, key: &str, label: &str, help: &str| {
        html! {
            label.choice {
                input type="radio" name="from" value=(key) checked[cfg.from == value];
                span { b { (label) } br; span.muted.small { (help) } }
            }
        }
    };
    html! {
        section.card.settings #notify {
            h2 { "Notifications" }
            p.muted.small {
                "Get alerts on your phone or in a chat channel, without opening this page. "
                "For your phone, install the ntfy app, subscribe to a topic name only you know, "
                "and add " code { "https://ntfy.sh/" i { "that-name" } } " here."
            }
            @if cfg.targets.is_empty() {
                p.small { "Alerts aren't sent anywhere yet." }
            } @else {
                ul.list {
                    @for t in &cfg.targets {
                        @let id = notify::target_id(t);
                        @let st = state.targets.get(&id);
                        li.row {
                            div.grow {
                                b { (t.kind.label()) } " " span.mono { (masked(&t.url)) }
                                div.sub {
                                    @if !running.contains(t) {
                                        "Sends alerts once Keepword restarts."
                                    } @else if let Some(f) = st.and_then(|s| s.failing.as_ref()) {
                                        "Failing since " (view::when(f.since_ms)) ": " (f.error)
                                    } @else if let Some(at) = st.and_then(|s| s.last_sent_ms) {
                                        "Last sent " (view::when(at))
                                    } @else {
                                        "Nothing sent yet."
                                    }
                                }
                            }
                            (view::button(ui, "/settings/notify/test", &[("id", &id)], "Send test", "ghost small", None))
                            (view::button(ui, "/settings/notify/remove", &[("id", &id)], "Remove", "ghost small",
                                Some("Stop sending alerts there?")))
                        }
                    }
                }
            }
            form.stack method="post" action="/settings/notify/add" {
                (view::csrf(ui))
                div.row-fields {
                    label { "Send to"
                        select name="kind" {
                            option value="ntfy" { "ntfy (phone)" }
                            option value="discord" { "Discord webhook" }
                            option value="slack" { "Slack webhook" }
                            option value="webhook" { "Other webhook (JSON)" }
                        }
                    }
                    label { "Address"
                        input type="url" name="url" required placeholder="https://ntfy.sh/your-topic" autocomplete="off";
                    }
                }
                details {
                    summary { "Access token" }
                    label { "Only if the server asks for one"
                        input type="password" name="token" autocomplete="off";
                    }
                }
                div { button.btn type="submit" { "Add" } }
            }
            details {
                summary { "Which alerts" }
                form.stack method="post" action="/settings/notify" {
                    (view::csrf(ui))
                    div.choices {
                        (from(NotifyFrom::ThisNode, "this-node", "Raised by this node",
                            "What this node saw itself."))
                        (from(NotifyFrom::Network, "network", "Also those other witnesses raised",
                            "Every alert this node hears of. Two of your nodes that know each other each send them."))
                    }
                    (kind_box(AlertKind::SilentEdit, "silent_edit", "Silent edits"))
                    (kind_box(AlertKind::Split, "split", "Different versions"))
                    (kind_box(AlertKind::Equivocation, "equivocation", "Log equivocations"))
                    label { "This web UI's address, to link alerts to it (optional)"
                        input type="url" name="ui_url" value=[cfg.ui_url.as_deref()] placeholder="https://keepword.example.org" autocomplete="off";
                    }
                    div { button.btn type="submit" { "Save" } }
                }
            }
        }
    }
}

/// All of `keepword.toml`, to edit as text.
fn file_editor(ui: &Ui, file: &str, open: bool) -> Markup {
    html! {
        details.card #file open[open] {
            summary { b { "Edit keepword.toml" } span.muted.small { " (every setting, for experts)" } }
            p.muted.small {
                "The whole configuration. It's checked before it's saved; see docs/keepword.example.toml for what each setting does."
            }
            form.stack method="post" action="/settings/file" {
                (view::csrf(ui))
                textarea.mono name="toml" rows="20" spellcheck="false" { (file) }
                div { button.btn type="submit" { "Save file" } }
            }
        }
    }
}

/// Restarting Keepword, to apply saved settings or unstick it: a button
/// where a service manager starts it again, else what to do instead.
pub(super) fn restart_control(ui: &Ui, primary: bool) -> Markup {
    html! {
        @if ui.opts.supervised {
            (view::button(ui, "/settings/restart", &[], "Restart Keepword",
                if primary { "primary small" } else { "small" },
                Some("Restart Keepword now? It is back in a few seconds.")))
        } @else {
            span.muted.small {
                "No service manager (systemd, Docker) runs Keepword here to start it again, so it \
                 can't restart itself: stop it and start it again. If one does run it, set "
                code { "KEEPWORD_SUPERVISED=1" } "."
            }
        }
    }
}

fn password_section(ui: &Ui) -> Markup {
    let set_form = |current: bool| {
        html! {
            form.stack method="post" action="/settings/password" {
                (view::csrf(ui))
                @if current {
                    label { "Current password" input type="password" name="current" autocomplete="current-password" required; }
                }
                div.row-fields {
                    label { "New password" input type="password" name="password" autocomplete="new-password" minlength="8" required; }
                    label { "Again" input type="password" name="again" autocomplete="new-password" minlength="8" required; }
                }
                div { button.btn type="submit" { @if current { "Change password" } @else { "Set password" } } }
            }
        }
    };
    html! {
        @match ui.auth.mode() {
            UiAuth::Off => {
                p { "No password: Keepword was started with " code { "--ui-auth none" } ", for a proxy in front of it that asks for one." }
            }
            UiAuth::LocalOnly => {
                p { "No password: this dashboard only answers at localhost. Set one to protect it anyway." }
                (set_form(false))
            }
            UiAuth::Password if ui.auth.password_from_env() => {
                p { "The password is set by " code { "KEEPWORD_UI_PASSWORD" } "; change it there." }
            }
            UiAuth::Password => {
                (set_form(true))
            }
        }
    }
}

pub(super) async fn save(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let dir = &ui.node.dir;
    let mut cfg = Config::load(dir)?;
    let country = form.get("country").to_ascii_uppercase();
    cfg.vantage.country = (!country.is_empty()).then_some(country);
    let asn = form.get("asn").trim_start_matches(['A', 'S', 'a', 's']);
    cfg.vantage.asn = match asn {
        "" => None,
        n => match n.parse() {
            Ok(n) => Some(n),
            Err(_) => {
                return Ok(flash(
                    "/settings#location",
                    "error",
                    "The ASN is a number, like 3320.",
                ));
            }
        },
    };
    let net = &mut cfg.network;
    if from_env("KEEPWORD_PEERS").is_none() {
        net.peers = form
            .get("peers")
            .split_whitespace()
            .map(str::to_string)
            .collect();
    }
    if from_env("KEEPWORD_SEEDS").is_none() {
        net.seeds = form.on("seeds");
    }
    if from_env("KEEPWORD_ENDPOINT").is_none() && from_env("KEEPWORD_RELAY").is_none() {
        match form.get("mode") {
            "alone" => {
                (net.endpoint, net.relay) = (None, None);
                net.peers.clear();
            }
            "relay" => (net.endpoint, net.relay) = (None, Some("auto".into())),
            "endpoint" => {
                let e = form.get("endpoint");
                if e.is_empty() {
                    return Ok(flash(
                        "/settings#network",
                        "error",
                        "Give the public address peers reach this node at.",
                    ));
                }
                (net.endpoint, net.relay) = (Some(e.to_string()), None);
            }
            // None chosen: as it was.
            _ => {}
        }
    }
    cfg.content.retain = match form.get("retain") {
        "full" => Retain::Full,
        "none" => Retain::None,
        _ => Retain::Normalized,
    };
    if let Ok(t) = form.get("timeout").parse::<u64>() {
        cfg.capture.timeout_secs = t.clamp(5, 300);
    }
    let ua = form.get("user_agent");
    cfg.capture.user_agent = (!ua.is_empty()).then(|| ua.to_string());
    if let Err(e) = cfg.validate() {
        return Ok(flash("/settings", "error", format!("Not saved: {e:#}")));
    }
    cfg.save(dir)?;
    // The banner under the heading says what applying them takes.
    Ok(flash("/settings", "ok", "Saved."))
}

pub(super) async fn save_file(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    // Browsers send line breaks as CRLF.
    let text = form.get("toml").replace("\r\n", "\n") + "\n";
    Ok(match Config::save_text(&ui.node.dir, &text) {
        Ok(()) => flash("/settings", "ok", "Saved."),
        Err(e) => flash("/settings#file", "error", format!("Not saved: {e:#}")),
    })
}

const NOTIFY: &str = "/settings#notify";

/// Change the saved notification settings, if they come out valid.
fn change_notify(ui: &Ui, ok: &str, f: impl FnOnce(&mut NotifyConfig)) -> WebResult<Response> {
    let dir = &ui.node.dir;
    let mut cfg = Config::load(dir)?;
    f(&mut cfg.notify);
    if let Err(e) = cfg.validate() {
        return Ok(flash(NOTIFY, "error", format!("Not saved: {e:#}")));
    }
    cfg.save(dir)?;
    Ok(flash(NOTIFY, "ok", ok))
}

pub(super) async fn notify_save(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    change_notify(&ui, "Saved.", |n| {
        n.from = match form.get("from") {
            "network" => NotifyFrom::Network,
            _ => NotifyFrom::ThisNode,
        };
        n.kinds = [
            (AlertKind::SilentEdit, "silent_edit"),
            (AlertKind::Split, "split"),
            (AlertKind::Equivocation, "equivocation"),
        ]
        .into_iter()
        .filter(|(_, name)| form.on(name))
        .map(|(k, _)| k)
        .collect();
        let u = form.get("ui_url");
        n.ui_url = (!u.is_empty()).then(|| u.to_string());
    })
}

pub(super) async fn notify_add(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let kind = match form.get("kind") {
        "discord" => TargetKind::Discord,
        "slack" => TargetKind::Slack,
        "webhook" => TargetKind::Webhook,
        _ => TargetKind::Ntfy,
    };
    let token = form.get("token");
    let target = NotifyTarget {
        kind,
        url: form.get("url").to_string(),
        token: (!token.is_empty()).then(|| token.to_string()),
    };
    change_notify(&ui, "Added. Send a test to see that it arrives.", |n| {
        if !n.targets.contains(&target) {
            n.targets.push(target);
        }
    })
}

pub(super) async fn notify_remove(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let id = form.get("id");
    change_notify(&ui, "Removed.", |n| {
        n.targets.retain(|t| notify::target_id(t) != id)
    })
}

pub(super) async fn notify_test(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    // As saved, so a target can be tried before a restart.
    let cfg = Config::load(&ui.node.dir)?;
    let Some(t) = cfg
        .notify
        .targets
        .iter()
        .find(|t| notify::target_id(t) == form.get("id"))
    else {
        return Ok(flash(NOTIFY, "error", "That target is gone."));
    };
    let sent = notify::send(&notify::client(&cfg)?, t, &notify::test_notice(&cfg.notify)).await;
    Ok(match sent {
        Ok(()) => flash(NOTIFY, "ok", format!("Sent to {}.", notify::target_name(t))),
        Err(e) => flash(NOTIFY, "error", format!("Not sent: {e:#}")),
    })
}

pub(super) async fn password(State(ui): State<Ui>, form: Post) -> WebResult<Response> {
    let back = "/settings#password";
    match ui.auth.mode() {
        UiAuth::Off => return Ok(flash(back, "error", "Passwords are off.")),
        UiAuth::Password if ui.auth.password_from_env() => {
            return Ok(flash(
                back,
                "error",
                "The password is set by KEEPWORD_UI_PASSWORD.",
            ));
        }
        UiAuth::Password if !check_password(&ui, form.raw("current")).await => {
            tokio::time::sleep(Duration::from_secs(1)).await;
            return Ok(flash(back, "error", "The current password isn't right."));
        }
        _ => {}
    }
    if let Err(msg) = check_new_password(form.raw("password"), form.raw("again")) {
        return Ok(flash(back, "error", msg));
    }
    store_password(&ui, form.raw("password")).await?;
    Ok(relogin(&ui, flash("/settings", "ok", "Password saved.")))
}

pub(super) async fn restart(State(ui): State<Ui>, _: Post) -> WebResult<Response> {
    // It wouldn't start again, and the UI would go with it.
    if let Err(e) = Config::load(&ui.node.dir) {
        return Ok(flash(
            "/settings",
            "error",
            format!(
                "Not restarting: keepword.toml isn't valid ({e:#}), and Keepword wouldn't start with it."
            ),
        ));
    }
    if !ui.opts.supervised {
        return Ok(flash(
            "/settings",
            "error",
            "Keepword wasn't started by a service manager that would start it again; restart it yourself.",
        ));
    }
    // After this answer is on its way.
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        RESTART.notify_one();
    });
    Ok(view::entry(
        "Restarting…",
        &None,
        html! {
            p { "Keepword is restarting with the new settings. This takes a few seconds." }
            p { a.btn.primary href="/settings" data-reload-after="6" { "Back to the settings" } }
        },
    )
    .into_response())
}
