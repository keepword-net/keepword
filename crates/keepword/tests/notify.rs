//! Alerts sent to the operator: each once, across restarts, and kept for a
//! target that is down without holding up the others.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use keepword::Node;
use keepword::config::{Config, NotifyTarget, TargetKind};
use keepword::notify::{self, State};
use keepword_core::net::AlertKind;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What a notification service was sent, and the status it answers with.
#[derive(Clone)]
struct Receiver {
    got: Arc<Mutex<Vec<(String, Value)>>>,
    status: Arc<AtomicU16>,
    base: String,
}

impl Receiver {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let r = Receiver {
            got: Default::default(),
            status: Arc::new(AtomicU16::new(200)),
            base: format!("http://{}", listener.local_addr().unwrap()),
        };
        let (got, status) = (r.got.clone(), r.status.clone());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let (got, status) = (got.clone(), status.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head, len) = loop {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..at]).to_string();
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            buf.drain(..at + 4);
                            break (head, len);
                        }
                    };
                    while buf.len() < len {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    // Where a 3xx sends a client, which always takes it.
                    let code = match path.as_str() {
                        "/moved" => 200,
                        _ => status.load(Ordering::SeqCst),
                    };
                    if code == 200 {
                        let body = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                        got.lock().unwrap().push((path, body));
                    }
                    let resp = format!(
                        "HTTP/1.1 {code} X\r\nLocation: /moved\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        r
    }

    fn take(&self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.got.lock().unwrap())
    }
}

fn target(kind: TargetKind, url: String) -> NotifyTarget {
    NotifyTarget {
        kind,
        url,
        token: None,
    }
}

fn node_with(dir: &std::path::Path, targets: Vec<NotifyTarget>) -> Node {
    let mut cfg = Config::default();
    cfg.network.seeds = false;
    cfg.beacon.drand_url = None;
    cfg.notify.targets = targets;
    Node::init(dir, &cfg).unwrap();
    Node::open(dir).unwrap()
}

async fn round(node: &Node) {
    let http = notify::client(&node.config).unwrap();
    notify::round(node, &http, &|l| eprintln!("{l}"))
        .await
        .unwrap();
}

fn silent_edit(node: &Node, page: &str) {
    node.raise_alert(
        AlertKind::SilentEdit,
        Some(page),
        "silent edit: +0 -2 lines".into(),
        vec![],
    )
    .unwrap();
}

#[tokio::test]
async fn alerts_reach_ntfy_once_across_restarts() {
    let rx = Receiver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let topic = target(TargetKind::Ntfy, format!("{}/my-alerts", rx.base));
    let node = node_with(dir.path(), vec![topic.clone()]);
    // Older than the target: not sent.
    silent_edit(&node, "https://old.example/privacy");
    round(&node).await;
    assert!(rx.take().is_empty(), "alerts from before it was set up");

    silent_edit(&node, "https://www.example.org/privacy");
    round(&node).await;
    let got = rx.take();
    assert_eq!(got.len(), 1, "{got:?}");
    let (path, m) = &got[0];
    assert_eq!(path, "/");
    assert_eq!(m["topic"], "my-alerts");
    assert_eq!(m["title"], "Silent edit: example.org/privacy");
    assert_eq!(m["click"], "https://www.example.org/privacy");

    // Nothing new, nothing sent; not after a restart either. The same
    // alert again, raised by the next capture, isn't sent again.
    round(&node).await;
    drop(node);
    let node = Node::open(dir.path()).unwrap();
    silent_edit(&node, "https://www.example.org/privacy");
    round(&node).await;
    assert!(rx.take().is_empty());

    let state = State::load(dir.path());
    let st = &state.targets[&notify::target_id(&topic)];
    assert!(st.failing.is_none() && st.last_sent_ms.is_some());
    // The file doesn't give the topic away.
    let file = std::fs::read_to_string(dir.path().join(notify::STATE_FILE)).unwrap();
    assert!(!file.contains("my-alerts"));
}

#[tokio::test]
async fn a_target_that_is_down_gets_its_alerts_later() {
    let (up, down) = (Receiver::start().await, Receiver::start().await);
    let dir = tempfile::tempdir().unwrap();
    let slack = target(TargetKind::Slack, format!("{}/services/x", down.base));
    let node = node_with(
        dir.path(),
        vec![
            target(TargetKind::Discord, format!("{}/api/webhooks/1/t", up.base)),
            slack.clone(),
        ],
    );
    round(&node).await;
    down.status.store(500, Ordering::SeqCst);
    silent_edit(&node, "https://a.example/terms");
    round(&node).await;
    assert_eq!(up.take().len(), 1, "the other target still gets it");
    let mut state = State::load(dir.path());
    let id = notify::target_id(&slack);
    let f = state.targets[&id].failing.clone().expect("failing");
    assert!(f.error.contains("HTTP 500"), "{}", f.error);
    assert!(
        !f.error.contains("/services/x"),
        "the URL is a secret: {}",
        f.error
    );

    // Not tried again before it is due.
    down.status.store(200, Ordering::SeqCst);
    round(&node).await;
    assert!(down.take().is_empty());
    // Due: everything it missed.
    state
        .targets
        .get_mut(&id)
        .unwrap()
        .failing
        .as_mut()
        .unwrap()
        .retry_at_ms = 0;
    state.save(dir.path()).unwrap();
    silent_edit(&node, "https://b.example/terms");
    round(&node).await;
    let got = down.take();
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got[0].1["text"]
            .as_str()
            .unwrap()
            .contains("a.example/terms")
    );
    assert!(State::load(dir.path()).targets[&id].failing.is_none());
    assert_eq!(up.take().len(), 1);
}

#[tokio::test]
async fn many_alerts_at_once_are_one_message() {
    let rx = Receiver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let node = node_with(
        dir.path(),
        vec![target(TargetKind::Webhook, format!("{}/hook", rx.base))],
    );
    round(&node).await;
    for i in 0..12 {
        silent_edit(&node, &format!("https://example.org/{i}"));
    }
    round(&node).await;
    let got = rx.take();
    assert_eq!(got.len(), 1);
    let (path, m) = &got[0];
    assert_eq!(path, "/hook");
    assert_eq!(m["title"], "12 new alerts");
    assert_eq!(m["alerts"].as_array().unwrap().len(), 12);
}

/// Followed, a redirect would turn the POST into a GET that looks like it
/// worked, and the alert would be lost.
#[tokio::test]
async fn a_redirect_is_a_failure_not_a_success() {
    let rx = Receiver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let topic = target(TargetKind::Ntfy, format!("{}/my-alerts", rx.base));
    let node = node_with(dir.path(), vec![topic.clone()]);
    round(&node).await;
    rx.status.store(301, Ordering::SeqCst);
    silent_edit(&node, "https://a.example/terms");
    round(&node).await;
    assert!(rx.take().is_empty(), "the redirect was followed");
    let st = &State::load(dir.path()).targets[&notify::target_id(&topic)];
    let f = st.failing.as_ref().expect("failing");
    assert!(f.error.contains("HTTP 301"), "{}", f.error);
    assert!(st.last_sent_ms.is_none());
}
