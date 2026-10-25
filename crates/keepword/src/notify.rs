//! Alerts sent where the operator sees them: an ntfy topic, a Discord or
//! Slack channel, or any webhook. `keepword serve` looks for new alerts
//! every half minute, however they came in (raised here, by another
//! witness, by a CLI command), and sends each target the ones it hasn't
//! had. Where each target is, and how sending to it goes, is kept in
//! `notify.state`, so a restart neither loses alerts nor sends them again.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use keepword_core::net::{Alert, AlertKind};
use keepword_core::statement::Signed;
use keepword_core::{WitnessKey, format_ms, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Node;
use crate::config::{Config, NotifyConfig, NotifyFrom, NotifyTarget, TargetKind};
use crate::httpc::{Http, RetryAfter};

pub const STATE_FILE: &str = "notify.state";

/// How often to look for new alerts.
const EVERY: Duration = Duration::from_secs(30);
/// The same alert again within this long isn't sent again: a page that
/// keeps changing back and forth raises one on every capture.
const FOLD_MS: i64 = 86_400_000;
/// More alerts than this at once go out as one notice.
const MAX_SINGLE: usize = 5;
/// Alerts looked at in one go.
const BATCH: u32 = 200;
/// A notice that fails for this long is skipped, so that one the service
/// refuses doesn't hold up the rest for good.
const GIVE_UP_MS: i64 = 86_400_000;
/// Longest wait between attempts at a failing target.
const MAX_BACKOFF_MS: i64 = 30 * 60_000;

/// Each target's progress, by `target_id`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub targets: HashMap<String, TargetState>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetState {
    /// The last alert dealt with, as `Store::alerts_after` numbers them.
    pub cursor: i64,
    /// When alerts were last sent, by `fold_key`.
    #[serde(default)]
    pub sent: HashMap<String, i64>,
    #[serde(default)]
    pub last_sent_ms: Option<i64>,
    #[serde(default)]
    pub failing: Option<Failing>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failing {
    pub since_ms: i64,
    pub attempts: u32,
    pub error: String,
    pub retry_at_ms: i64,
}

impl State {
    /// As saved; empty if it can't be read, so targets start over from the
    /// alerts to come.
    pub fn load(dir: &Path) -> State {
        std::fs::read(dir.join(STATE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(STATE_FILE);
        let tmp = path.with_extension("state.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// A target's name in `notify.state`, which doesn't keep its URL: whoever
/// has that can post there.
pub fn target_id(t: &NotifyTarget) -> String {
    blake3::hash(format!("{:?} {}", t.kind, t.url).as_bytes()).to_hex()[..16].to_string()
}

/// A target for logs and errors: its kind and host.
pub fn target_name(t: &NotifyTarget) -> String {
    let host = url::Url::parse(&t.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    format!("{} at {host}", t.kind.label())
}

/// One message: an alert, or several that came in at once.
#[derive(Debug)]
pub struct Notice {
    pub title: String,
    pub message: String,
    /// The page in the web UI, or else the page itself.
    pub link: Option<String>,
    /// An equivocation: a witness can't be trusted any more.
    pub urgent: bool,
    pub alerts: Vec<Signed<Alert>>,
}

/// What the test from Settings or `keepword notify test` sends.
pub fn test_notice(cfg: &NotifyConfig) -> Notice {
    Notice {
        title: "Keepword test".into(),
        message: "Alerts from this node reach you here.".into(),
        link: cfg.ui_url.clone(),
        urgent: false,
        alerts: vec![],
    }
}

fn kind_title(k: AlertKind) -> &'static str {
    match k {
        AlertKind::SilentEdit => "Silent edit",
        AlertKind::Split => "Different versions",
        AlertKind::Equivocation => "Log equivocation",
    }
}

/// A link from an alert, which another witness may have written, if it is
/// one to follow.
fn web_link(u: &str) -> Option<String> {
    let u = url::Url::parse(u).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.to_string())
}

/// A page's address as the web UI shows it: no scheme, no `www.`.
fn address(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else {
        return url.to_string();
    };
    let Some(host) = u.host_str() else {
        return url.to_string();
    };
    let mut s = host.trim_start_matches("www.").to_string();
    if let Some(p) = u.port() {
        s += &format!(":{p}");
    }
    if u.path() != "/" {
        s += u.path();
    }
    if let Some(q) = u.query() {
        s += &format!("?{q}");
    }
    s
}

/// A time for people: `22 Oct 2026, 14:43 UTC`.
fn when(ms: i64) -> String {
    let f = time::macros::format_description!(
        "[day padding:none] [month repr:short] [year], [hour]:[minute] UTC"
    );
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| t.format(&f).ok())
        .unwrap_or_else(|| format_ms(ms))
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// Alerts are the same, for folding, as the Alerts page folds them.
fn fold_key(a: &Alert) -> String {
    let key = json!([a.kind, a.url, a.summary, a.issuer.0]).to_string();
    blake3::hash(key.as_bytes()).to_hex()[..16].to_string()
}

fn ui_link(cfg: &NotifyConfig, path: &str) -> Option<String> {
    cfg.ui_url.as_deref().map(|u| crate::httpc::join(u, path))
}

fn single(cfg: &NotifyConfig, a: &Signed<Alert>, from: &str) -> Notice {
    let b = &a.body;
    let title = match &b.url {
        Some(u) => format!("{}: {}", kind_title(b.kind), address(u)),
        None => kind_title(b.kind).to_string(),
    };
    let prefix = format!("{}: ", b.kind.label());
    let summary = b.summary.strip_prefix(&prefix).unwrap_or(&b.summary);
    let link = match &b.url {
        Some(u) => ui_link(cfg, &format!("/page?u={}", urlencode(u))).or_else(|| web_link(u)),
        None => ui_link(cfg, "/alerts"),
    };
    Notice {
        title: clip(&title, 200),
        message: format!(
            "{}\nFrom {from}, {}",
            clip(summary, 1000),
            when(b.issued_at_ms)
        ),
        link,
        urgent: b.kind == AlertKind::Equivocation,
        alerts: vec![a.clone()],
    }
}

fn digest(cfg: &NotifyConfig, alerts: Vec<Signed<Alert>>) -> Notice {
    let mut lines: Vec<String> = alerts
        .iter()
        .take(MAX_SINGLE)
        .map(|a| match &a.body.url {
            Some(u) => clip(&format!("{}: {}", kind_title(a.body.kind), address(u)), 200),
            None => kind_title(a.body.kind).to_string(),
        })
        .collect();
    if alerts.len() > MAX_SINGLE {
        lines.push(format!("and {} more", alerts.len() - MAX_SINGLE));
    }
    Notice {
        title: format!("{} new alerts", alerts.len()),
        message: lines.join("\n"),
        link: ui_link(cfg, "/alerts"),
        urgent: alerts
            .iter()
            .any(|a| a.body.kind == AlertKind::Equivocation),
        alerts,
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// What to send a target for `alerts` (in the order they came in), each
/// notice with the last alert it covers and the alerts to mark sent: only
/// the alerts it is set to send, those sent within a day left out, and
/// many at once as one notice.
fn notices(
    cfg: &NotifyConfig,
    me: &WitnessKey,
    alerts: &[(i64, Signed<Alert>)],
    sent: &HashMap<String, i64>,
    now: i64,
    name: impl Fn(&WitnessKey) -> String,
) -> Vec<(i64, Notice, Vec<String>)> {
    let mut seen = HashSet::new();
    let due: Vec<_> = alerts
        .iter()
        .filter(|(_, a)| cfg.from == NotifyFrom::Network || a.body.issuer == *me)
        .filter(|(_, a)| cfg.kinds.contains(&a.body.kind))
        .filter_map(|(n, a)| {
            let key = fold_key(&a.body);
            let recent = sent.get(&key).is_some_and(|t| *t > now - FOLD_MS);
            (!recent && seen.insert(key.clone())).then_some((*n, a, key))
        })
        .collect();
    if due.len() > MAX_SINGLE {
        let upto = due.last().map_or(0, |d| d.0);
        let keys = due.iter().map(|d| d.2.clone()).collect();
        let alerts = due.into_iter().map(|d| d.1.clone()).collect();
        return vec![(upto, digest(cfg, alerts), keys)];
    }
    due.into_iter()
        .map(|(n, a, key)| (n, single(cfg, a, &name(&a.body.issuer)), vec![key]))
        .collect()
}

/// Markdown that Discord would act on, made plain text.
fn discord_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\*_~`|>[]()#<:-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What Slack reads as markup (links, `<!channel>`), made plain text.
fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Where to post a notice to a target, and the JSON to post.
fn payload(t: &NotifyTarget, n: &Notice) -> Result<(String, Vec<u8>)> {
    let (url, body) = match t.kind {
        // Published as JSON, which, unlike headers, takes any text.
        TargetKind::Ntfy => {
            let mut base = url::Url::parse(&t.url)?;
            let (dir, topic) = base
                .path()
                .rsplit_once('/')
                .map(|(d, t)| (d.to_string(), t.to_string()))
                .ok_or_else(|| anyhow!("an ntfy URL ends in the topic"))?;
            base.set_path(&format!("{dir}/"));
            base.set_query(None);
            let tags: &[&str] = match (n.alerts.is_empty(), n.urgent) {
                (true, _) => &[],
                (false, true) => &["rotating_light"],
                (false, false) => &["warning"],
            };
            let mut m = json!({
                "topic": topic,
                "title": n.title,
                "message": n.message,
                "tags": tags,
                "priority": if n.urgent { 4 } else { 3 },
            });
            if let Some(l) = &n.link {
                m["click"] = json!(l);
            }
            (base.to_string(), m)
        }
        TargetKind::Discord => {
            let mut embed = json!({
                "title": clip(&discord_escape(&n.title), 250),
                "description": clip(&discord_escape(&n.message), 4000),
                "color": if n.urgent { 0xd1_34_38 } else { 0xe8_a3_3d },
            });
            if let Some(l) = &n.link {
                embed["url"] = json!(l);
            }
            let m = json!({
                "username": "Keepword",
                "embeds": [embed],
                // Nobody is pinged, whatever the text says.
                "allowed_mentions": { "parse": [] },
            });
            (t.url.clone(), m)
        }
        TargetKind::Slack => {
            let title = slack_escape(&n.title);
            let head = match &n.link {
                Some(l) => format!("*<{}|{title}>*", l.replace('|', "%7C").replace('>', "%3E")),
                None => format!("*{title}*"),
            };
            let m = json!({
                "text": format!("{head}\n{}", slack_escape(&n.message)),
                "unfurl_links": false,
            });
            (t.url.clone(), m)
        }
        TargetKind::Webhook => {
            let m = json!({
                "title": n.title,
                "message": n.message,
                "link": n.link,
                "urgent": n.urgent,
                "alerts": n.alerts,
            });
            (t.url.clone(), m)
        }
    };
    Ok((url, serde_json::to_vec(&body)?))
}

pub fn client(cfg: &Config) -> Result<Http> {
    Http::for_notices(cfg.capture.use_system_proxy)
}

pub async fn send(http: &Http, t: &NotifyTarget, n: &Notice) -> Result<()> {
    let (url, body) = payload(t, n)?;
    http.post_notice(&url, t.token.as_deref(), body).await
}

fn witness_name(node: &Node, k: &WitnessKey) -> String {
    if *k == node.key.public() {
        return "this node".into();
    }
    let ep = node.store.peer(k).ok().flatten().and_then(|p| p.endpoint);
    match ep.as_deref().and_then(crate::relay::endpoint_host) {
        Some((h, false)) => h,
        Some((h, true)) => format!("witness {} through {h}", k.short()),
        None => format!("witness {}", k.short()),
    }
}

pub async fn notify_loop(node: Arc<Node>, log: impl Fn(String)) -> Result<()> {
    let http = client(&node.config)?;
    loop {
        // Like a failed sync, a failed round is tried again.
        if let Err(e) = round(&node, &http, &log).await {
            log(format!(
                "{}  notifications failed: {e:#}",
                format_ms(now_ms())
            ));
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// Send every target the alerts it hasn't had.
pub async fn round(node: &Node, http: &Http, log: &impl Fn(String)) -> Result<()> {
    let mut state = State::load(&node.dir);
    let before = state.clone();
    let top = node.store.alerts_last()?;
    let now = now_ms();
    let mut targets = HashMap::new();
    for t in &node.config.notify.targets {
        let id = target_id(t);
        // A new target gets the alerts to come, not the last 180 days'.
        let mut st = state.targets.remove(&id).unwrap_or(TargetState {
            cursor: top,
            ..Default::default()
        });
        // Past the last one: the newest were pruned, and their numbers
        // come again.
        st.cursor = st.cursor.min(top);
        st.sent.retain(|_, at| *at > now - FOLD_MS);
        // What the others sent is kept all the same.
        if st.failing.as_ref().is_none_or(|f| f.retry_at_ms <= now)
            && let Err(e) = deliver(node, http, t, &mut st, log).await
        {
            log(format!(
                "{}  notify {}: {e:#}",
                format_ms(now_ms()),
                target_name(t)
            ));
        }
        targets.insert(id, st);
    }
    // Those no longer set are forgotten.
    state.targets = targets;
    if state != before {
        state.save(&node.dir)?;
    }
    Ok(())
}

async fn deliver(
    node: &Node,
    http: &Http,
    t: &NotifyTarget,
    st: &mut TargetState,
    log: &impl Fn(String),
) -> Result<()> {
    let cfg = &node.config.notify;
    let me = node.key.public();
    let mut count = 0;
    'batches: loop {
        let alerts = node.store.alerts_after(st.cursor, BATCH)?;
        let Some(&(last, _)) = alerts.last() else {
            break;
        };
        let now = now_ms();
        for (upto, notice, keys) in
            notices(cfg, &me, &alerts, &st.sent, now, |k| witness_name(node, k))
        {
            match send(http, t, &notice).await {
                Ok(()) => {
                    st.cursor = upto;
                    st.failing = None;
                    st.last_sent_ms = Some(now_ms());
                    st.sent.extend(keys.into_iter().map(|k| (k, now)));
                    count += notice.alerts.len();
                }
                Err(e) => {
                    let now = now_ms();
                    let f = st.failing.get_or_insert(Failing {
                        since_ms: now,
                        attempts: 0,
                        error: String::new(),
                        retry_at_ms: now,
                    });
                    f.attempts += 1;
                    f.error = format!("{e:#}");
                    let backoff =
                        (30_000i64 << f.attempts.min(10).saturating_sub(1)).min(MAX_BACKOFF_MS);
                    let asked = e.downcast_ref::<RetryAfter>().map_or(0, |r| {
                        i64::try_from(r.0).unwrap_or(i64::MAX).saturating_mul(1000)
                    });
                    f.retry_at_ms = now.saturating_add(backoff.max(asked));
                    log(format!(
                        "{}  notify {}: {e:#}",
                        format_ms(now),
                        target_name(t)
                    ));
                    if now - f.since_ms < GIVE_UP_MS {
                        break 'batches;
                    }
                    // Skip it, and give the rest a day of their own.
                    log(format!(
                        "{}  notify {}: gave up on {} alerts after a day",
                        format_ms(now),
                        target_name(t),
                        notice.alerts.len()
                    ));
                    st.cursor = upto;
                    f.since_ms = now;
                    f.retry_at_ms = now;
                }
            }
        }
        st.cursor = last;
        if alerts.len() < BATCH as usize {
            break;
        }
    }
    if count > 0 {
        log(format!(
            "{}  sent {count} alert{} to {}",
            format_ms(now_ms()),
            if count == 1 { "" } else { "s" },
            target_name(t)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepword_core::Keypair;

    fn alert(
        k: &Keypair,
        kind: AlertKind,
        url: Option<&str>,
        summary: &str,
        at: i64,
    ) -> Signed<Alert> {
        Signed::sign(
            Alert {
                kind,
                url: url.map(str::to_string),
                summary: summary.into(),
                evidence: vec![],
                issued_at_ms: at,
                issuer: k.public(),
            },
            k,
        )
        .unwrap()
    }

    fn target(kind: TargetKind, url: &str) -> NotifyTarget {
        NotifyTarget {
            kind,
            url: url.into(),
            token: None,
        }
    }

    fn body(t: &NotifyTarget, n: &Notice) -> (String, serde_json::Value) {
        let (url, b) = payload(t, n).unwrap();
        (url, serde_json::from_slice(&b).unwrap())
    }

    #[test]
    fn only_new_alerts_of_this_node_once_a_day() {
        let (me, other) = (Keypair::generate().unwrap(), Keypair::generate().unwrap());
        let cfg = NotifyConfig::default();
        let now = 1_800_000_000_000;
        let page = Some("https://www.example.org/privacy");
        let alerts = vec![
            (
                1,
                alert(
                    &me,
                    AlertKind::SilentEdit,
                    page,
                    "silent edit: +0 -2 lines",
                    now,
                ),
            ),
            (
                2,
                alert(
                    &other,
                    AlertKind::SilentEdit,
                    page,
                    "silent edit: +1 -0 lines",
                    now,
                ),
            ),
            // The same again on the next capture.
            (
                3,
                alert(
                    &me,
                    AlertKind::SilentEdit,
                    page,
                    "silent edit: +0 -2 lines",
                    now + 1,
                ),
            ),
            (
                4,
                alert(
                    &me,
                    AlertKind::Equivocation,
                    None,
                    "witness ab signed two logs",
                    now,
                ),
            ),
        ];
        let name = |_: &WitnessKey| "this node".to_string();
        let got = notices(&cfg, &me.public(), &alerts, &HashMap::new(), now, name);
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), [1, 4]);
        let n = &got[0].1;
        assert_eq!(n.title, "Silent edit: example.org/privacy");
        assert!(
            n.message.starts_with("+0 -2 lines\nFrom this node, "),
            "{}",
            n.message
        );
        assert_eq!(n.link.as_deref(), Some("https://www.example.org/privacy"));
        assert!(got[1].1.urgent && !n.urgent);

        // Sent a day ago or less: not again. Longer ago: again.
        let sent: HashMap<_, _> = got[0]
            .2
            .iter()
            .map(|k| (k.clone(), now - FOLD_MS / 2))
            .collect();
        let again = notices(&cfg, &me.public(), &alerts[..1], &sent, now, name);
        assert!(again.is_empty());
        let later = notices(&cfg, &me.public(), &alerts[..1], &sent, now + FOLD_MS, name);
        assert_eq!(later.len(), 1);

        // From the whole network, and only the kinds asked for.
        let cfg = NotifyConfig {
            from: NotifyFrom::Network,
            kinds: vec![AlertKind::SilentEdit],
            ui_url: Some("https://node.example/".into()),
            ..Default::default()
        };
        let got = notices(&cfg, &me.public(), &alerts, &HashMap::new(), now, name);
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(
            got[0].1.link.as_deref(),
            Some("https://node.example/page?u=https%3A%2F%2Fwww.example.org%2Fprivacy")
        );
    }

    #[test]
    fn many_at_once_are_one_notice() {
        let me = Keypair::generate().unwrap();
        let alerts: Vec<_> = (1..=8)
            .map(|i| {
                let url = format!("https://example.org/{i}");
                (
                    i,
                    alert(&me, AlertKind::SilentEdit, Some(&url), "silent edit: x", 0),
                )
            })
            .collect();
        let cfg = NotifyConfig::default();
        let got = notices(&cfg, &me.public(), &alerts, &HashMap::new(), 0, |_| {
            String::new()
        });
        assert_eq!(got.len(), 1);
        let (upto, n, keys) = &got[0];
        assert_eq!((*upto, keys.len(), n.alerts.len()), (8, 8, 8));
        assert_eq!(n.title, "8 new alerts");
        assert!(
            n.message
                .ends_with("Silent edit: example.org/5\nand 3 more"),
            "{}",
            n.message
        );
        assert_eq!(n.link, None);
    }

    #[test]
    fn what_other_witnesses_wrote_stays_text() {
        let k = Keypair::generate().unwrap();
        let a = alert(
            &k,
            AlertKind::Split,
            Some("javascript:alert(1)"),
            "split: @everyone <!channel> [click](https://evil.example)",
            0,
        );
        let n = single(&NotifyConfig::default(), &a, "witness x");
        assert_eq!(n.link, None, "not a link to follow");

        let (_, d) = body(
            &target(TargetKind::Discord, "https://discord.com/api/webhooks/1/t"),
            &n,
        );
        assert_eq!(d["allowed_mentions"]["parse"], json!([]));
        let text = d["embeds"][0]["description"].as_str().unwrap();
        assert!(
            text.contains(r"\[click\]\(https\://evil.example\)"),
            "{text}"
        );
        assert!(text.contains(r"\<!channel\>"), "{text}");

        let (_, s) = body(
            &target(TargetKind::Slack, "https://hooks.slack.com/services/x"),
            &n,
        );
        let text = s["text"].as_str().unwrap();
        assert!(
            text.contains("&lt;!channel&gt;") && !text.contains("<!channel>"),
            "{text}"
        );
    }

    #[test]
    fn ntfy_is_published_as_json_to_the_server() {
        let n = Notice {
            title: "Silent edit: ünïcode.example".into(),
            message: "+1 -1 lines".into(),
            link: Some("https://node.example/alerts".into()),
            urgent: false,
            alerts: vec![alert(
                &Keypair::generate().unwrap(),
                AlertKind::SilentEdit,
                None,
                "",
                0,
            )],
        };
        let (url, m) = body(&target(TargetKind::Ntfy, "https://ntfy.sh/my-alerts"), &n);
        assert_eq!(url, "https://ntfy.sh/");
        assert_eq!(m["topic"], "my-alerts");
        assert_eq!(m["title"], "Silent edit: ünïcode.example");
        assert_eq!(m["click"], "https://node.example/alerts");
        assert_eq!(m["tags"], json!(["warning"]));
        // Under a path, behind a proxy.
        let (url, m) = body(
            &target(TargetKind::Ntfy, "https://home.example/ntfy/alerts"),
            &n,
        );
        assert_eq!(
            (url.as_str(), &m["topic"]),
            ("https://home.example/ntfy/", &json!("alerts"))
        );
        // A webhook gets the signed alerts, to check them.
        let (url, w) = body(&target(TargetKind::Webhook, "https://hook.example/in"), &n);
        assert_eq!(url, "https://hook.example/in");
        assert!(w["alerts"][0]["body"]["issuer"].is_string());
    }
}
