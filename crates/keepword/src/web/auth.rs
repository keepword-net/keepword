//! Who may use the UI, and requests it can trust.
//!
//! - A password, stored as an Argon2 hash in `ui-password`, or given in
//!   `KEEPWORD_UI_PASSWORD`. A login is a cookie signed with the key in
//!   `ui-secret`, over the password's hash: a new password logs everyone
//!   out.
//! - Every form carries a token only the UI's own pages know, and the
//!   browser must say a form came from them: no other site can make a
//!   logged-in browser act.
//! - Without a password, the UI only answers requests addressed to
//!   localhost: a page that points its own domain at 127.0.0.1 (DNS
//!   rebinding) is turned away.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::{Form, FromRequest, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use keepword_core::now_ms;
use maud::html;

use super::{Ui, view};

/// Who must log in to the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAuth {
    /// Everyone, with the password.
    Password,
    /// Nobody, and the UI only answers at localhost: for a UI that only
    /// listens on this machine.
    LocalOnly,
    /// Nobody: a proxy in front of the UI does the login.
    Off,
}

const PASSWORD_FILE: &str = "ui-password";
const SECRET_FILE: &str = "ui-secret";
const SESSION: &str = "keepword_session";
const FLASH: &str = "keepword_flash";
const SESSION_DAYS: i64 = 30;

pub(super) struct Auth {
    /// As started; see `mode`.
    configured: UiAuth,
    dir: PathBuf,
    secret: [u8; 32],
    /// The hash of `KEEPWORD_UI_PASSWORD`, which then is the password.
    env_hash: Option<String>,
    /// One login attempt at a time, each failure a second long.
    attempts: tokio::sync::Mutex<()>,
}

impl Auth {
    pub fn new(dir: &Path, mode: UiAuth) -> Result<Self> {
        let env = std::env::var("KEEPWORD_UI_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty());
        Ok(Auth {
            configured: mode,
            dir: dir.to_path_buf(),
            secret: load_secret(&dir.join(SECRET_FILE))?,
            env_hash: env.map(|p| hash(&p)).transpose()?,
            attempts: Default::default(),
        })
    }

    /// Who must log in now: a UI that only answers at localhost asks for a
    /// password once one is set, and stops when its file is deleted.
    pub fn mode(&self) -> UiAuth {
        match self.configured {
            UiAuth::LocalOnly if self.has_password() => UiAuth::Password,
            m => m,
        }
    }

    /// The password's hash, if one is set.
    fn password_hash(&self) -> Option<String> {
        self.env_hash.clone().or_else(|| {
            std::fs::read_to_string(self.dir.join(PASSWORD_FILE))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    }

    pub fn has_password(&self) -> bool {
        self.password_hash().is_some()
    }

    /// The password comes from `KEEPWORD_UI_PASSWORD`, so the UI can't
    /// change it.
    pub fn password_from_env(&self) -> bool {
        self.env_hash.is_some()
    }

    pub fn verify(&self, password: &str) -> bool {
        self.password_hash().is_some_and(|h| {
            PasswordHash::new(&h).is_ok_and(|h| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &h)
                    .is_ok()
            })
        })
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        write_private(&self.dir.join(PASSWORD_FILE), hash(password)?.as_bytes())
    }

    fn mac(&self, what: &str) -> blake3::Hash {
        blake3::keyed_hash(&self.secret, what.as_bytes())
    }

    /// The token every form carries.
    pub fn csrf(&self) -> String {
        self.mac("csrf").to_hex().to_string()
    }

    fn csrf_ok(&self, token: &str) -> bool {
        // `Hash`'s equality takes the same time whatever differs.
        blake3::Hash::from_hex(token).is_ok_and(|t| t == self.mac("csrf"))
    }

    fn session_mac(&self, expires: i64, password_hash: &str) -> blake3::Hash {
        self.mac(&format!("session {expires} {password_hash}"))
    }

    fn session_cookie(&self) -> Option<String> {
        let expires = now_ms() + SESSION_DAYS * 86_400_000;
        let mac = self.session_mac(expires, &self.password_hash()?);
        Some(format!(
            "{SESSION}={expires}.{}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict",
            mac.to_hex(),
            SESSION_DAYS * 86_400
        ))
    }

    fn logged_in(&self, headers: &HeaderMap) -> bool {
        let (Some(c), Some(h)) = (cookie(headers, SESSION), self.password_hash()) else {
            return false;
        };
        let Some((expires, mac)) = c.split_once('.') else {
            return false;
        };
        let Ok(expires) = expires.parse::<i64>() else {
            return false;
        };
        expires > now_ms()
            && blake3::Hash::from_hex(mac).is_ok_and(|m| m == self.session_mac(expires, &h))
    }
}

fn hash(password: &str) -> Result<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hashing the password: {e}"))?
        .to_string())
}

/// The key that signs logins, made on first start.
fn load_secret(path: &Path) -> Result<[u8; 32]> {
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(bytes) = hex::decode(text.trim())
        && let Ok(secret) = <[u8; 32]>::try_from(bytes)
    {
        return Ok(secret);
    }
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    write_private(path, hex::encode(secret).as_bytes())?;
    Ok(secret)
}

/// Write a file only its owner can read, replacing it whole.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// Whether a request is addressed to this machine by name or address: its
/// Host header, or over HTTP/2 the authority in its URI.
fn local_host(req: &Request) -> bool {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri().authority().map(|a| a.as_str()));
    let Some(host) = host else {
        // HTTP/1.0 clients, curl with no Host: nothing to rebind.
        return true;
    };
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    let name = name.to_ascii_lowercase();
    name == "localhost"
        || name.ends_with(".localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// A one-time message for the next page: what a form did.
#[derive(Clone, Default)]
pub(super) struct Flash(pub Option<(String, String)>);

/// Go to `to` after a form, with a message: `kind` is `ok` or `error`.
pub(super) fn flash(to: &str, kind: &str, msg: impl AsRef<str>) -> Response {
    let msg: String = msg.as_ref().chars().take(600).collect();
    let value: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, &msg)
        .finish();
    let mut r = redirect(to);
    if let Ok(v) = HeaderValue::from_str(&format!(
        "{FLASH}={value}; Path=/; Max-Age=60; HttpOnly; SameSite=Strict"
    )) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

pub(super) fn redirect(to: &str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, to.to_string())]).into_response()
}

fn read_flash(headers: &HeaderMap) -> Option<(String, String)> {
    let v = cookie(headers, FLASH)?;
    url::form_urlencoded::parse(v.as_bytes())
        .next()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
}

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
    connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// Every request: the address check, the login, flash messages and
/// security headers.
pub(super) async fn guard(State(ui): State<Ui>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let asset = path.starts_with("/static/") || path == "/favicon.svg";
    let mut res = if ui.auth.mode() == UiAuth::LocalOnly && !local_host(&req) {
        (
            StatusCode::MISDIRECTED_REQUEST,
            view::bare(
                "Open this at localhost",
                "Without a password, Keepword's dashboard only answers at localhost, so that \
                 no website can reach it through your browser. Open it as http://localhost \
                 (through an SSH tunnel if it runs elsewhere), or start Keepword with \
                 --ui-auth password.",
            ),
        )
            .into_response()
    } else if ui.auth.mode() == UiAuth::Password
        && !asset
        && let Some(to) = login_needed(&ui, &req, &path)
    {
        redirect(&to)
    } else {
        let flash = read_flash(req.headers());
        let had_flash = flash.is_some();
        req.extensions_mut().insert(Flash(flash));
        let mut res = next.run(req).await;
        let sets_flash = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|c| c.as_bytes().starts_with(FLASH.as_bytes()));
        if had_flash && !sets_flash {
            res.headers_mut().append(
                header::SET_COOKIE,
                HeaderValue::from_static("keepword_flash=; Path=/; Max-Age=0"),
            );
        }
        res
    };
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

/// Where to send a request that needs a login first, or a password set up.
fn login_needed(ui: &Ui, req: &Request, path: &str) -> Option<String> {
    if !ui.auth.has_password() {
        return (path != "/setup").then(|| "/setup".to_string());
    }
    if path == "/login" || path == "/setup" || ui.auth.logged_in(req.headers()) {
        return None;
    }
    let here = req
        .uri()
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_string();
    Some(format!("/login?next={}", view::q(&here)))
}

/// A form posted from the UI's own pages, its fields by name.
pub(super) struct Post(HashMap<String, String>);

impl Post {
    /// A field, trimmed; empty when missing.
    pub fn get(&self, name: &str) -> &str {
        self.0.get(name).map_or("", |v| v.trim())
    }

    /// A checkbox.
    pub fn on(&self, name: &str) -> bool {
        !self.get(name).is_empty()
    }
}

impl FromRequest<Ui> for Post {
    type Rejection = Response;

    async fn from_request(req: Request, ui: &Ui) -> Result<Self, Response> {
        let forbidden = |msg: &str| (StatusCode::FORBIDDEN, view::bare("Form refused", msg));
        // Browsers say where a request comes from; old ones say nothing.
        if let Some(site) = req.headers().get("sec-fetch-site")
            && site != "same-origin"
            && site != "none"
        {
            return Err(forbidden("This form came from another website.").into_response());
        }
        let Form(fields) = Form::<HashMap<String, String>>::from_request(req, ui)
            .await
            .map_err(IntoResponse::into_response)?;
        if !fields.get("csrf").is_some_and(|t| ui.auth.csrf_ok(t)) {
            return Err(forbidden(
                "This form has expired. Go back, reload the page and try again.",
            )
            .into_response());
        }
        Ok(Post(fields))
    }
}

/// Where to go after logging in: a page of the UI, never another site.
fn next_page(next: Option<&String>) -> String {
    next.filter(|n| n.starts_with('/') && !n.starts_with("//") && !n.starts_with("/\\"))
        .cloned()
        .unwrap_or_else(|| "/".into())
}

pub(super) async fn login_page(
    State(ui): State<Ui>,
    Query(q): Query<HashMap<String, String>>,
    axum::Extension(Flash(flash)): axum::Extension<Flash>,
) -> Response {
    if ui.auth.mode() != UiAuth::Password {
        return redirect("/");
    }
    let next = next_page(q.get("next"));
    view::entry(
        "Log in",
        &flash,
        html! {
            form.stack method="post" action="/login" {
                (view::csrf(&ui))
                input type="hidden" name="next" value=(next);
                label { "Password"
                    input type="password" name="password" autocomplete="current-password" required autofocus;
                }
                button.btn.primary type="submit" data-busy="Checking…" { "Log in" }
            }
            p.muted.small {
                "Forgot it? Delete the file " code { (PASSWORD_FILE) } " in Keepword's data folder and \
                 open this page again to choose a new one."
            }
        },
    )
    .into_response()
}

pub(super) async fn login(State(ui): State<Ui>, form: Post) -> Response {
    let _one_at_a_time = ui.auth.attempts.lock().await;
    let pw = form.0.get("password").map_or("", String::as_str);
    let next = next_page(form.0.get("next"));
    if ui.auth.verify(pw)
        && let Some(c) = ui.auth.session_cookie()
    {
        let mut r = redirect(&next);
        if let Ok(v) = HeaderValue::from_str(&c) {
            r.headers_mut().append(header::SET_COOKIE, v);
        }
        return r;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    flash(
        &format!("/login?next={}", view::q(&next)),
        "error",
        "That password isn't right.",
    )
}

pub(super) async fn logout(_: Post) -> Response {
    let mut r = redirect("/login");
    r.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static("keepword_session=; Path=/; Max-Age=0"),
    );
    r
}

pub(super) async fn setup_page(
    State(ui): State<Ui>,
    axum::Extension(Flash(flash)): axum::Extension<Flash>,
) -> Response {
    if ui.auth.mode() != UiAuth::Password {
        return redirect("/");
    }
    if ui.auth.has_password() {
        return redirect("/login");
    }
    view::entry(
        "Welcome to Keepword",
        &flash,
        html! {
            p { "Choose a password for this dashboard. Anyone who can reach it with the \
                 password can change what this node watches and how it runs." }
            form.stack method="post" action="/setup" {
                (view::csrf(&ui))
                label { "Password"
                    input type="password" name="password" autocomplete="new-password" minlength="8" required autofocus;
                }
                label { "Again"
                    input type="password" name="again" autocomplete="new-password" minlength="8" required;
                }
                button.btn.primary type="submit" { "Set password and continue" }
            }
        },
    )
    .into_response()
}

pub(super) async fn setup(State(ui): State<Ui>, form: Post) -> Response {
    if ui.auth.mode() != UiAuth::Password || ui.auth.has_password() {
        return redirect("/");
    }
    let pw = form.0.get("password").map_or("", String::as_str);
    if let Err(msg) = check_new_password(pw, form.0.get("again").map_or("", String::as_str)) {
        return flash("/setup", "error", msg);
    }
    if let Err(e) = ui.auth.set_password(pw) {
        return flash("/setup", "error", format!("{e:#}"));
    }
    let mut r = flash("/", "ok", "Password set. Welcome!");
    if let Some(c) = ui.auth.session_cookie()
        && let Ok(v) = HeaderValue::from_str(&c)
    {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

/// Why a new password won't do, if it won't.
pub(super) fn check_new_password(pw: &str, again: &str) -> Result<(), &'static str> {
    if pw.chars().count() < 8 {
        Err("Use at least 8 characters.")
    } else if pw != again {
        Err("The two passwords differ.")
    } else {
        Ok(())
    }
}

/// A new login cookie, after the password changed.
pub(super) fn relogin(ui: &Ui, mut r: Response) -> Response {
    if let Some(c) = ui.auth.session_cookie()
        && let Ok(v) = HeaderValue::from_str(&c)
    {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addressed_to(host: &str) -> Request {
        Request::get("/")
            .header(header::HOST, host)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[test]
    fn local_hosts() {
        for ok in [
            "localhost",
            "localhost:8480",
            "127.0.0.1:8480",
            "127.1.2.3",
            "[::1]:8480",
            "app.localhost",
        ] {
            assert!(local_host(&addressed_to(ok)), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:8480",
            "localhost.evil.example",
            "192.168.1.5:8480",
            "[2001:db8::1]:80",
        ] {
            assert!(!local_host(&addressed_to(bad)), "{bad}");
        }
        // HTTP/2: the authority, in the URI.
        let h2 = Request::get("http://evil.example/")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(!local_host(&h2));
    }

    #[test]
    fn next_pages_stay_on_the_ui() {
        let n = |s: &str| next_page(Some(&s.to_string()));
        assert_eq!(n("/pages?show=review"), "/pages?show=review");
        for bad in [
            "//evil.example",
            "https://evil.example",
            "/\\evil.example",
            "",
        ] {
            assert_eq!(n(bad), "/");
        }
    }

    #[test]
    fn sessions_end_with_the_password() {
        let t = tempfile::tempdir().unwrap();
        let a = Auth::new(t.path(), UiAuth::Password).unwrap();
        assert!(a.session_cookie().is_none());
        a.set_password("correct horse").unwrap();
        assert!(a.verify("correct horse") && !a.verify("wrong horse"));
        let mut h = HeaderMap::new();
        let c = a.session_cookie().unwrap();
        let value = c.split(';').next().unwrap();
        h.insert(header::COOKIE, HeaderValue::from_str(value).unwrap());
        assert!(a.logged_in(&h));
        a.set_password("battery staple").unwrap();
        assert!(!a.logged_in(&h));
        // The key survives a restart; the token is the same.
        let b = Auth::new(t.path(), UiAuth::Password).unwrap();
        assert_eq!(a.csrf(), b.csrf());
        assert!(b.csrf_ok(&a.csrf()) && !b.csrf_ok("00"));
    }
}
