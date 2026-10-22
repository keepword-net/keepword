//! The web UI, through its router: who may use it, its forms, and what it
//! shows of a change.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use keepword::Node;
use keepword::config::{Config, Retain};
use keepword::web::{Options, UiAuth};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

fn node(dir: &std::path::Path) -> Arc<Node> {
    let mut cfg = Config::default();
    cfg.capture.allow_private_addresses = true;
    cfg.content.retain = Retain::Normalized;
    cfg.beacon.drand_url = None;
    Node::init(dir, &cfg).unwrap();
    Arc::new(Node::open(dir).unwrap())
}

fn ui(node: &Arc<Node>, auth: UiAuth) -> Router {
    keepword::web::router(
        node.clone(),
        Options {
            auth,
            watching: true,
            supervised: false,
        },
    )
    .unwrap()
}

/// A page whose body can change between requests.
async fn site(body: Arc<Mutex<String>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let body = body.lock().unwrap().clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => n += k,
                    }
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/policy")
}

struct Answer {
    status: StatusCode,
    location: String,
    cookies: Vec<String>,
    body: String,
}

async fn send(app: &Router, req: Request<Body>) -> Answer {
    let res = app.clone().oneshot(req).await.unwrap();
    let h = res.headers();
    let location = h
        .get(header::LOCATION)
        .map_or("", |v| v.to_str().unwrap())
        .to_string();
    let cookies = h
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|c| c.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        location,
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, path: &str, cookie: &str) -> Answer {
    let req = Request::get(path)
        .header(header::HOST, "localhost:8480")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

async fn post(app: &Router, path: &str, cookie: &str, fields: &[(&str, &str)]) -> Answer {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let req = Request::post(path)
        .header(header::HOST, "localhost:8480")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "same-origin")
        .body(Body::from(body))
        .unwrap();
    send(app, req).await
}

/// The token in a page's forms.
fn csrf(html: &str) -> String {
    let at = html.find("name=\"csrf\" value=\"").expect("a form") + 19;
    html[at..at + 64].to_string()
}

#[tokio::test]
async fn a_local_ui_only_answers_at_localhost() {
    let t = tempfile::tempdir().unwrap();
    let app = ui(&node(t.path()), UiAuth::LocalOnly);
    assert_eq!(get(&app, "/", "").await.status, StatusCode::OK);
    // A page that pointed its own domain at 127.0.0.1.
    let req = Request::get("/")
        .header(header::HOST, "evil.example:8480")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&app, req).await.status,
        StatusCode::MISDIRECTED_REQUEST
    );
}

#[tokio::test]
async fn forms_need_the_ui_s_own_token() {
    let t = tempfile::tempdir().unwrap();
    let n = node(t.path());
    let app = ui(&n, UiAuth::LocalOnly);
    let token = csrf(&get(&app, "/", "").await.body);
    let forged = "0".repeat(64);
    let add = [
        ("urls", "https://a.example/terms"),
        ("every", "3600"),
        ("csrf", forged.as_str()),
    ];
    assert_eq!(
        post(&app, "/pages/add", "", &add).await.status,
        StatusCode::FORBIDDEN
    );
    let mut ok = add;
    ok[2].1 = &token;
    // Not from another site, whatever its token.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(ok)
        .finish();
    let cross = Request::post("/pages/add")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "cross-site")
        .body(Body::from(body))
        .unwrap();
    assert_eq!(send(&app, cross).await.status, StatusCode::FORBIDDEN);
    assert!(n.store.watches().unwrap().is_empty());
    let a = post(&app, "/pages/add", "", &ok).await;
    assert_eq!(a.status, StatusCode::SEE_OTHER);
    let w = n.store.watches().unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(
        (w[0].url.as_str(), w[0].every_secs),
        ("https://a.example/terms", 3600)
    );
    // The message shows once, on the next page.
    let flash = a.cookies.join("; ");
    assert!(
        get(&app, &a.location, &flash)
            .await
            .body
            .contains("Watching https://a.example/terms")
    );
}

#[tokio::test]
async fn a_password_is_set_up_then_asked_for() {
    let t = tempfile::tempdir().unwrap();
    let app = ui(&node(t.path()), UiAuth::Password);
    let first = get(&app, "/", "").await;
    assert_eq!(
        (first.status, first.location.as_str()),
        (StatusCode::SEE_OTHER, "/setup")
    );
    let token = csrf(&get(&app, "/setup", "").await.body);
    let short = post(
        &app,
        "/setup",
        "",
        &[("csrf", &token), ("password", "short"), ("again", "short")],
    )
    .await;
    assert_eq!(short.location, "/setup");
    let set = post(
        &app,
        "/setup",
        "",
        &[
            ("csrf", &token),
            ("password", "correct horse"),
            ("again", "correct horse"),
        ],
    )
    .await;
    let session = set
        .cookies
        .iter()
        .find(|c| c.starts_with("keepword_session="))
        .unwrap()
        .clone();
    assert_eq!(get(&app, "/pages", &session).await.status, StatusCode::OK);
    let out = get(&app, "/pages?show=review", "").await;
    assert_eq!(out.location, "/login?next=%2Fpages%3Fshow%3Dreview");
    let wrong = post(
        &app,
        "/login",
        "",
        &[
            ("csrf", &token),
            ("password", "wrong horse"),
            ("next", "/pages"),
        ],
    )
    .await;
    assert!(
        wrong
            .cookies
            .iter()
            .all(|c| !c.starts_with("keepword_session="))
    );
    let right = post(
        &app,
        "/login",
        "",
        &[
            ("csrf", &token),
            ("password", "correct horse"),
            ("next", "/pages"),
        ],
    )
    .await;
    assert_eq!(right.location, "/pages");
    let session = right
        .cookies
        .iter()
        .find(|c| c.starts_with("keepword_session="))
        .unwrap();
    assert_eq!(get(&app, "/settings", session).await.status, StatusCode::OK);
    // Assets need no login: the login page uses them.
    assert_eq!(
        get(&app, "/static/app.css", "").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_change_shows_what_changed_and_is_reviewed() {
    let t = tempfile::tempdir().unwrap();
    let n = node(t.path());
    let app = ui(&n, UiAuth::LocalOnly);
    let body = Arc::new(Mutex::new(
        "<html><head><title>Policy</title></head><body><main><p>We keep logs for 30 days.</p></main></body></html>".to_string(),
    ));
    let url = site(body.clone()).await;
    n.capture(&url, false).await.unwrap();
    *body.lock().unwrap() =
        "<html><head><title>Policy</title></head><body><main><p>We keep logs for 3 years.</p>\
        <p>&lt;script&gt;alert(1)&lt;/script&gt;</p></main></body></html>"
            .to_string();
    n.capture(&url, false).await.unwrap();
    let home = get(&app, "/", "").await;
    assert!(home.body.contains("Silent edit"), "{}", home.body);
    assert!(
        home.body.contains("<mark class=\"del\">30</mark>"),
        "{}",
        home.body
    );
    assert!(home.body.contains("<mark class=\"ins\">3</mark>"));
    // The page's text is text, never markup.
    assert!(!home.body.contains("<script>alert"));
    assert!(home.body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    let seq = n.store.changes_to_review(1).unwrap()[0].seq;
    for view in ["", "?view=whole", "?view=lines"] {
        let c = get(&app, &format!("/change/{seq}{view}"), "").await;
        assert_eq!(c.status, StatusCode::OK, "{view}");
        assert!(!c.body.contains("<script>alert"), "{view}");
    }
    let page = get(
        &app,
        &format!(
            "/page?u={}",
            url::form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>()
        ),
        "",
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Policy"));
    let token = csrf(&home.body);
    post(
        &app,
        "/review",
        "",
        &[("csrf", &token), ("seq", &seq.to_string())],
    )
    .await;
    assert_eq!(n.store.changes_to_review_count().unwrap(), 0);
    // Nothing is watched, so the overview says how to start.
    let home = get(&app, "/", "").await.body;
    assert!(!home.contains("Mark as reviewed") && home.contains("Start by watching a page"));
    // Old links still lead there.
    let old = get(&app, &format!("/url?u={url}"), "").await;
    assert!(old.location.starts_with("/page?u="));
}

#[tokio::test]
async fn checking_now_and_stopping() {
    let t = tempfile::tempdir().unwrap();
    let n = node(t.path());
    let app = ui(&n, UiAuth::LocalOnly);
    let url = site(Arc::new(Mutex::new("<p>Terms</p>".into()))).await;
    let token = csrf(&get(&app, "/", "").await.body);
    post(
        &app,
        "/pages/add",
        "",
        &[("csrf", &token), ("urls", &url), ("every", "86400")],
    )
    .await;
    let a = post(&app, "/page/check", "", &[("csrf", &token), ("u", &url)]).await;
    assert_eq!(a.status, StatusCode::SEE_OTHER);
    assert_eq!(n.store.history(&url).unwrap().len(), 1);
    assert!(n.store.watches().unwrap()[0].last_run.is_some());
    post(&app, "/page/unwatch", "", &[("csrf", &token), ("u", &url)]).await;
    assert!(n.store.watches().unwrap().is_empty());
    // Too often is refused.
    let r = post(
        &app,
        "/pages/add",
        "",
        &[("csrf", &token), ("urls", &url), ("every", "5")],
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_change_without_its_content_still_shows() {
    let t = tempfile::tempdir().unwrap();
    let n = node(t.path());
    let app = ui(&n, UiAuth::LocalOnly);
    let body = Arc::new(Mutex::new("<p>one</p>".to_string()));
    let url = site(body.clone()).await;
    n.capture(&url, false).await.unwrap();
    *body.lock().unwrap() = "<p>two</p>".into();
    n.capture(&url, false).await.unwrap();
    let c = n.store.changes_to_review(1).unwrap().remove(0);
    let page = format!("/change/{}", c.seq);
    // An erasure request: the content goes, the records stay.
    n.store.purge(&url, false).unwrap();
    assert!(
        get(&app, "/", "")
            .await
            .body
            .contains("didn't keep the pages' text")
    );
    let shown = get(&app, &page, "").await;
    assert!(shown.body.contains("keeps no text"), "{}", shown.body);
    assert!(!shown.body.contains("read the same"));
    let diff = format!("/diff?from={}&to={}", c.to_id, c.from_id);
    assert!(get(&app, &diff, "").await.body.contains("keeps no text"));
    // The records too, as `keepword purge --forget` in another process
    // would while this page is drawn.
    let db = rusqlite::Connection::open(t.path().join("index.sqlite")).unwrap();
    db.execute("DELETE FROM attestations", []).unwrap();
    let home = get(&app, "/", "").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(
        home.body.contains("no longer has these captures"),
        "{}",
        home.body
    );
    let gone = get(&app, &page, "").await;
    assert_eq!(gone.status, StatusCode::OK);
    assert!(gone.body.contains("no longer has these captures"));
}

#[tokio::test]
async fn settings_are_saved_checked_and_kept_as_written() {
    let t = tempfile::tempdir().unwrap();
    let n = node(t.path());
    let app = ui(&n, UiAuth::LocalOnly);
    let token = csrf(&get(&app, "/settings", "").await.body);
    let form = |asn: &'static str| {
        vec![
            ("country", "de"),
            ("asn", asn),
            ("mode", "relay"),
            ("seeds", "on"),
            ("peers", "https://a.example\nhttps://b.example"),
            ("retain", "full"),
            ("timeout", "1000"),
            ("user_agent", ""),
        ]
    };
    let mut bad = form("Telekom");
    bad.push(("csrf", &token));
    post(&app, "/settings", "", &bad).await;
    assert!(Config::load(t.path()).unwrap().vantage.asn.is_none());
    let mut ok = form("AS3320");
    ok.push(("csrf", &token));
    let saved = post(&app, "/settings", "", &ok).await;
    assert_eq!(saved.location, "/settings");
    let c = Config::load(t.path()).unwrap();
    assert_eq!(c.vantage.country.as_deref(), Some("DE"));
    assert_eq!(c.vantage.asn, Some(3320));
    assert_eq!(c.network.relay.as_deref(), Some("auto"));
    assert_eq!(c.network.peers, ["https://a.example", "https://b.example"]);
    assert_eq!(c.content.retain, Retain::Full);
    assert_eq!(c.capture.timeout_secs, 300);
    // The node runs on what it read at start: a restart is pending.
    assert!(
        get(&app, "/settings", "")
            .await
            .body
            .contains("take effect after a restart")
    );
    // The file as written, comments and all, once it checks out.
    let file = t.path().join("keepword.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    let typo = format!("{before}\n[network]\nendpont = \"x\"\n");
    post(
        &app,
        "/settings/file",
        "",
        &[("csrf", &token), ("toml", &typo)],
    )
    .await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let commented = format!("# kept\r\n{before}");
    post(
        &app,
        "/settings/file",
        "",
        &[("csrf", &token), ("toml", &commented)],
    )
    .await;
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("# kept\n")
    );
}

#[tokio::test]
async fn a_password_is_used_as_typed() {
    let t = tempfile::tempdir().unwrap();
    let app = ui(&node(t.path()), UiAuth::LocalOnly);
    let token = csrf(&get(&app, "/settings", "").await.body);
    let pw = " spaced out ";
    let set = post(
        &app,
        "/settings/password",
        "",
        &[("csrf", &token), ("password", pw), ("again", pw)],
    )
    .await;
    // Now it asks, and the cookie from setting it logs in.
    let session = set
        .cookies
        .iter()
        .find(|c| c.starts_with("keepword_session="))
        .unwrap()
        .clone();
    assert_eq!(get(&app, "/", "").await.location, "/login?next=%2F");
    assert_eq!(get(&app, "/", &session).await.status, StatusCode::OK);
    let trimmed = post(
        &app,
        "/login",
        "",
        &[("csrf", &token), ("password", pw.trim())],
    )
    .await;
    assert!(
        trimmed
            .cookies
            .iter()
            .all(|c| !c.starts_with("keepword_session="))
    );
    let typed = post(&app, "/login", "", &[("csrf", &token), ("password", pw)]).await;
    assert!(
        typed
            .cookies
            .iter()
            .any(|c| c.starts_with("keepword_session="))
    );
}
