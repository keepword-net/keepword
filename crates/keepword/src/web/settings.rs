//! Settings: the ones most nodes change, as a form; the whole of
//! `keepword.toml` for the rest; the password; and a restart to apply
//! them, since the node reads its settings when it starts.

use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use maud::{Markup, html};

use super::auth::{
    Post, UiAuth, check_new_password, check_password, flash, relogin, store_password,
};
use super::view::{self, Ctx, Nav};
use super::{RESTART, Ui, WebResult};
use crate::config::{CONFIG_FILE, Config, Retain};

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
    let saved = Config::load(&node.dir)?;
    let pending = toml::to_string(&saved)? != toml::to_string(&node.config)?;
    let file = std::fs::read_to_string(node.dir.join(CONFIG_FILE)).unwrap_or_default();
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
            section.card #password {
                h2 { "Password" }
                (password_section(&ui))
            }
            details.card #file {
                summary { b { "Edit keepword.toml" } span.muted.small { " (every setting, for experts)" } }
                p.muted.small {
                    "The whole configuration. It's checked before it's saved; see docs/keepword.example.toml for what each setting does."
                }
                form.stack method="post" action="/settings/file" {
                    (view::csrf(&ui))
                    textarea.mono name="toml" rows="20" spellcheck="false" { (file) }
                    div { button.btn type="submit" { "Save file" } }
                }
            }
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
