//! Relays, for witnesses that can't accept connections (at home, behind
//! NAT). Such a witness advertises `<relay>/v1/relay/<key>` as its endpoint
//! and keeps polls open to the relay over its own connection; the relay
//! hands it the requests peers send to that address and returns its
//! answers.
//!
//! A relay stores nothing and can't forge anything: what peers fetch is
//! signed by the witness. It can only withhold, which looks like the
//! witness being offline. Requests through a relay carry no client
//! address, so the witness observes nobody through it; peers still observe
//! the witness, whose own pushes reach them directly.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Path, RawQuery, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use keepword_core::assign::Candidate;
use keepword_core::net::{Descriptor, RelayPoll};
use keepword_core::statement::Signed;
use keepword_core::{Digest, WitnessKey, now_ms};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tower::ServiceExt;

use crate::Node;
use crate::api::{ApiError, ApiResult, client_ip, client_net, not_found};
use crate::federation::SKEW_MS;
use crate::httpc::{MAX_RESPONSE, join};

/// A peer's request waits this long for the witness's answer, and a poll
/// this long for a request: both well within the 20 s peers wait.
const ANSWER_WAIT: Duration = Duration::from_secs(10);
const POLL_WAIT: Duration = Duration::from_secs(15);
/// A witness that hasn't polled for this long is offline.
const OFFLINE_AFTER: Duration = Duration::from_secs(60);
/// Requests queued for one witness, and polls it keeps open.
const QUEUE: usize = 32;
const POLLERS: usize = 4;
/// Witnesses relayed for one address (an IPv6 /64 is one).
const PER_ADDRESS: usize = 2;
/// Polls in a row that may fail before a witness gives its relay up.
const RETRIES: u64 = 3;

/// A peer's request, handed to the witness.
#[derive(Serialize, Deserialize)]
pub(crate) struct Relayed {
    id: u64,
    method: String,
    /// Path and query, e.g. `/v1/gossip?after=7`.
    path: String,
    content_type: Option<String>,
    /// Base64.
    body: String,
}

/// The witness's answer to one.
#[derive(Clone, Serialize, Deserialize)]
struct Answer {
    id: u64,
    status: u16,
    content_type: Option<String>,
    body: String,
}

/// A poll, with the answer to the request the last one got.
#[derive(Serialize, Deserialize)]
pub(crate) struct Poll {
    poll: Signed<RelayPoll>,
    answer: Option<Answer>,
}

/// The witnesses this node relays for, while they keep polling, and the
/// relay it is reached through itself.
#[derive(Default)]
pub(crate) struct Relay {
    slots: Mutex<HashMap<WitnessKey, Slot>>,
    ids: AtomicU64,
    pub(crate) via: Mutex<Option<String>>,
    /// Where the relay places witnesses: through it, this node observes
    /// nobody itself.
    pub(crate) view: Mutex<HashMap<WitnessKey, Candidate>>,
}

struct Slot {
    requests: mpsc::Sender<Relayed>,
    next: Arc<tokio::sync::Mutex<mpsc::Receiver<Relayed>>>,
    answers: HashMap<u64, oneshot::Sender<Answer>>,
    polled: Instant,
    /// Where it polls from.
    net: IpAddr,
}

impl Relay {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<WitnessKey, Slot>> {
        self.slots.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Place witnesses where the relay does.
    pub(crate) fn set_view(&self, candidates: Vec<Candidate>) {
        *self.view.lock().unwrap_or_else(|p| p.into_inner()) =
            candidates.into_iter().map(|c| (c.key, c)).collect();
    }
}

/// The relay a witness was last reached through and where it placed
/// witnesses, saved each sync: commands run beside the service can't ask
/// the relay, and the service starting again places witnesses at once.
#[derive(Serialize, Deserialize)]
pub struct Saved {
    pub relay: String,
    pub candidates: Vec<Candidate>,
}

const SAVED: &str = "relay.json";

impl Saved {
    pub fn load(dir: &std::path::Path) -> Option<Saved> {
        serde_json::from_slice(&std::fs::read(dir.join(SAVED)).ok()?).ok()
    }

    pub(crate) fn save(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        // Replaced whole, so a command never reads half of it.
        let tmp = dir.join(format!("{SAVED}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, dir.join(SAVED))?;
        Ok(())
    }
}

impl Slot {
    fn new(net: IpAddr) -> Self {
        let (requests, rx) = mpsc::channel(QUEUE);
        Slot {
            requests,
            next: Arc::new(tokio::sync::Mutex::new(rx)),
            answers: HashMap::new(),
            polled: Instant::now(),
            net,
        }
    }
}

fn status(code: StatusCode, msg: &str) -> ApiError {
    ApiError(code, msg.into())
}

/// A relayed witness polls: pass on its answer, then wait for a request.
pub(crate) async fn poll(
    State(node): State<Arc<Node>>,
    Path(key): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(p): Json<Poll>,
) -> ApiResult<Option<Relayed>> {
    let net = client_net(client_ip(&node.config.network, addr.ip(), &headers));
    let max = node.config.network.relay_witnesses;
    let key: WitnessKey = key.parse().map_err(|_| not_found())?;
    if max == 0 {
        return Err(not_found());
    }
    let b = &p.poll.body;
    let now = now_ms();
    if p.poll.verify().is_err()
        || b.witness != key
        || b.relay != node.key.public()
        || b.sent_at_ms.abs_diff(now) > SKEW_MS as u64
        || !node.fresh_envelope(p.poll.id(), now)
    {
        return Err(status(
            StatusCode::FORBIDDEN,
            "not a fresh poll by this witness",
        ));
    }
    let next = {
        let mut slots = node.relay.slots();
        if let Some(a) = p.answer
            && let Some(tx) = slots.get_mut(&key).and_then(|s| s.answers.remove(&a.id))
        {
            let _ = tx.send(a);
        }
        slots.retain(|_, s| s.polled.elapsed() < OFFLINE_AFTER);
        let Some(slot) = slots.get_mut(&key) else {
            // A cap per address, so keys made up by one person can't fill it.
            if slots.len() >= max || slots.values().filter(|s| s.net == net).count() >= PER_ADDRESS
            {
                return Err(status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "this relay is full",
                ));
            }
            // Taken: say so at once, the witness waits for this to advertise
            // the relay.
            slots.insert(key, Slot::new(net));
            return Ok(Json(None));
        };
        slot.polled = Instant::now();
        slot.next.clone()
    };
    // Waiting behind the witness's other polls counts too: each must return
    // before the witness gives up on it.
    Ok(Json(
        tokio::time::timeout(POLL_WAIT, async { next.lock().await.recv().await })
            .await
            .ok()
            .flatten(),
    ))
}

/// A peer's request to a relayed witness: queue it, wait for the answer.
pub(crate) async fn forward(
    State(node): State<Arc<Node>>,
    Path((key, path)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let key: WitnessKey = key.parse().map_err(|_| not_found())?;
    let path = path.trim_start_matches('/');
    // The witness's own API, not a relay of its own.
    if !path.starts_with("v1/") || path.starts_with("v1/relay/") {
        return Err(not_found());
    }
    // Base64 in a poll answer, which must stay under what peers read.
    if body.len() > MAX_RESPONSE / 2 {
        return Err(status(StatusCode::PAYLOAD_TOO_LARGE, "too large to relay"));
    }
    let id = node.relay.ids.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
        let mut slots = node.relay.slots();
        let slot = slots
            .get_mut(&key)
            .filter(|s| s.polled.elapsed() < OFFLINE_AFTER)
            .ok_or_else(|| {
                status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "witness not connected to this relay",
                )
            })?;
        let request = Relayed {
            id,
            method: method.to_string(),
            path: match query {
                Some(q) => format!("/{path}?{q}"),
                None => format!("/{path}"),
            },
            content_type: headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            body: B64.encode(&body),
        };
        slot.requests.try_send(request).map_err(|_| {
            status(
                StatusCode::SERVICE_UNAVAILABLE,
                "too many requests waiting for this witness",
            )
        })?;
        slot.answers.insert(id, tx);
    }
    let Ok(Ok(a)) = tokio::time::timeout(ANSWER_WAIT, rx).await else {
        if let Some(s) = node.relay.slots().get_mut(&key) {
            s.answers.remove(&id);
        }
        return Err(status(
            StatusCode::GATEWAY_TIMEOUT,
            "the witness didn't answer",
        ));
    };
    let body = B64
        .decode(&a.body)
        .map_err(|_| status(StatusCode::BAD_GATEWAY, "bad answer"))?;
    let code = StatusCode::from_u16(a.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut resp = (code, body).into_response();
    if let Some(ct) = a.content_type.and_then(|c| c.parse().ok()) {
        resp.headers_mut().insert(CONTENT_TYPE, ct);
    }
    Ok(resp)
}

/// Serve this node's API through `network.relay`.
pub async fn client(node: Arc<Node>, log: impl Fn(String)) -> anyhow::Result<()> {
    let stamp = AtomicI64::new(0);
    loop {
        for relay in candidates(&node)? {
            if let Err(e) = serve_via(&node, &relay, &stamp, &log).await {
                log(format!("relay {relay}: {e:#}"));
            }
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// `network.relay`, or with `auto` this node's peers that can accept
/// connections, in an order of its own: witnesses spread over the relays,
/// and each returns to the same one.
fn candidates(node: &Node) -> anyhow::Result<Vec<String>> {
    let net = &node.config.network;
    if let Some(r) = net.relay.as_ref().filter(|r| *r != "auto") {
        return Ok(vec![r.clone()]);
    }
    let mut relays: Vec<String> = node
        .store
        .peers()?
        .into_iter()
        .filter_map(|p| p.endpoint)
        .filter(|e| !e.contains("/v1/relay/"))
        .collect();
    if relays.is_empty() {
        relays = net.bootstrap();
    }
    let me = node.key.public();
    relays
        .sort_by_cached_key(|r| Digest::tagged("keepword relay-order v1", &[&me.0, r.as_bytes()]));
    Ok(relays)
}

/// Poll `relay` until it fails. The first poll it accepts makes it this
/// node's endpoint.
async fn serve_via(
    node: &Arc<Node>,
    relay: &str,
    stamp: &AtomicI64,
    log: &impl Fn(String),
) -> anyhow::Result<()> {
    let d: Signed<Descriptor> = node.net.get_json(&join(relay, "/v1/descriptor")).await?;
    d.verify()?;
    let url = join(relay, &format!("/v1/relay/{}", node.key.public()));
    let mut first = Some(poll_once(node, &url, d.body.key, stamp, None).await?);
    *node.relay.via.lock().unwrap_or_else(|p| p.into_inner()) = Some(relay.to_string());
    *node.descriptor.lock().unwrap_or_else(|p| p.into_inner()) = node.build_descriptor()?;
    log(format!("reached through the relay {relay}"));
    let api = crate::api::router(node.clone());
    let pollers =
        (0..POLLERS).map(|_| poller(node, &api, &url, d.body.key, stamp, first.take().flatten()));
    futures::future::try_join_all(pollers).await.map(|_| ())
}

/// Answer the request a poll got with the next poll, until polls keep
/// failing.
async fn poller(
    node: &Node,
    api: &Router,
    url: &str,
    relay: WitnessKey,
    stamp: &AtomicI64,
    mut next: Option<Relayed>,
) -> anyhow::Result<()> {
    let mut answer = None;
    let mut failed = 0;
    loop {
        if let Some(r) = next.take() {
            answer = Some(handle(api, r).await);
        }
        match poll_once(node, url, relay, stamp, answer.clone()).await {
            Ok(n) => {
                next = n;
                answer = None;
                failed = 0;
            }
            // A blip, or the relay restarting: try again, with the answer.
            Err(_) if failed < RETRIES => {
                failed += 1;
                tokio::time::sleep(Duration::from_secs(2 * failed)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn poll_once(
    node: &Node,
    url: &str,
    relay: WitnessKey,
    stamp: &AtomicI64,
    answer: Option<Answer>,
) -> anyhow::Result<Option<Relayed>> {
    // Each poll is signed at a distinct time, now or just after: a relay
    // refuses one it has seen.
    stamp.fetch_max(now_ms() - 1, Ordering::SeqCst);
    let sent_at_ms = stamp.fetch_add(1, Ordering::SeqCst) + 1;
    let poll = RelayPoll {
        witness: node.key.public(),
        relay,
        sent_at_ms,
    };
    let poll = Poll {
        poll: Signed::sign(poll, &node.key)?,
        answer,
    };
    node.net.post_json(url, &poll).await
}

/// Answer a relayed request with this node's own API.
async fn handle(api: &Router, r: Relayed) -> Answer {
    let mut answer = Answer {
        id: r.id,
        status: StatusCode::BAD_GATEWAY.as_u16(),
        content_type: None,
        body: String::new(),
    };
    let Ok(body) = B64.decode(&r.body) else {
        return answer;
    };
    let mut req = axum::http::Request::builder()
        .method(r.method.as_str())
        .uri(&r.path);
    if let Some(ct) = &r.content_type {
        req = req.header(CONTENT_TYPE, ct);
    }
    let Ok(mut req) = req.body(Body::from(body)) else {
        return answer;
    };
    // An unknown address: nobody is observed through a relay.
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([0, 0, 0, 0], 0))));
    let Ok(resp) = api.clone().oneshot(req).await;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    if let Ok(b) = axum::body::to_bytes(resp.into_body(), MAX_RESPONSE).await {
        answer.status = status;
        answer.content_type = content_type;
        answer.body = B64.encode(b);
    }
    answer
}
