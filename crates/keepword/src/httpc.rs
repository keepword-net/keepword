//! HTTP client for peers and external services (drand, OpenTimestamps
//! calendars, Esplora). Responses are size-limited, and peer endpoints come
//! from untrusted descriptors, so private addresses are refused unless the
//! node is configured for LAN use.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use keepword_capture::netpolicy::{PublicOnlyResolver, is_public};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    allow_private: bool,
}

const MAX_REDIRECTS: usize = 5;

fn builder(use_system_proxy: bool) -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder()
        .user_agent(concat!("keepword/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10));
    if use_system_proxy { b } else { b.no_proxy() }
}

impl Http {
    pub fn new(allow_private: bool, use_system_proxy: bool) -> Result<Self> {
        let mut b = builder(use_system_proxy);
        if allow_private {
            b = b.redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS));
        } else {
            // The resolver only sees host names; IP literals, including
            // those in redirects, are checked here.
            b = b.dns_resolver(Arc::new(PublicOnlyResolver)).redirect(
                reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= MAX_REDIRECTS {
                        attempt.error("too many redirects")
                    } else if let Err(e) = check_public(attempt.url()) {
                        attempt.error(e.to_string())
                    } else {
                        attempt.follow()
                    }
                }),
            );
        }
        Ok(Http {
            client: b.build()?,
            allow_private,
        })
    }

    /// For notification services. Their URLs are the operator's, so they
    /// may be on this network, like a self-hosted ntfy. A redirect isn't
    /// followed: it would turn the POST into a GET that looks like it
    /// worked.
    pub fn for_notices(use_system_proxy: bool) -> Result<Self> {
        Ok(Http {
            client: builder(use_system_proxy)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            allow_private: true,
        })
    }

    /// Refuse non-HTTP schemes and, unless allowed, private IP literals.
    fn check(&self, url: &str) -> Result<()> {
        let u = reqwest::Url::parse(url).with_context(|| format!("bad URL {url:?}"))?;
        if u.scheme() != "http" && u.scheme() != "https" {
            bail!("{url}: only http(s) URLs are allowed");
        }
        if !self.allow_private {
            check_public(&u)?;
        }
        Ok(())
    }

    async fn read(resp: reqwest::Response, max: usize) -> Result<Vec<u8>> {
        let status = resp.status();
        let mut resp = resp;
        if resp.content_length().is_some_and(|l| l as usize > max) {
            bail!("response too large");
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > max {
                bail!("response too large");
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let text = String::from_utf8_lossy(&body[..body.len().min(300)]).into_owned();
            return Err(anyhow!(HttpStatus(status.as_u16(), text)));
        }
        Ok(body)
    }

    pub async fn get_bytes(&self, url: &str, max: usize) -> Result<Vec<u8>> {
        self.check(url)?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        Self::read(resp, max)
            .await
            .with_context(|| format!("GET {url}"))
    }

    pub async fn post_bytes(
        &self,
        url: &str,
        body: Vec<u8>,
        accept: &str,
        max: usize,
    ) -> Result<Vec<u8>> {
        self.check(url)?;
        let resp = self
            .client
            .post(url)
            .header(reqwest::header::ACCEPT, accept)
            .body(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        Self::read(resp, max)
            .await
            .with_context(|| format!("POST {url}"))
    }

    /// POST to a notification service, which answers with nothing worth
    /// reading. Whoever has the URL can post there too, so errors only
    /// name its host.
    pub async fn post_notice(&self, url: &str, token: Option<&str>, body: Vec<u8>) -> Result<()> {
        let u = reqwest::Url::parse(url).map_err(|_| anyhow!("not a URL"))?;
        let host = u.host_str().unwrap_or("?").to_string();
        if !self.allow_private {
            check_public(&u)?;
        }
        let mut req = self
            .client
            .post(u)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!(e.without_url()))
            .with_context(|| format!("POST to {host}"))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
            && let Some(secs) = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()?.trim().parse().ok())
        {
            return Err(anyhow!(RetryAfter(secs))).with_context(|| format!("POST to {host}"));
        }
        Self::read(resp, 64 * 1024)
            .await
            .map(drop)
            .map_err(|e| anyhow!("{}", e.root_cause()))
            .with_context(|| format!("POST to {host}"))
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let b = self.get_bytes(url, MAX_RESPONSE).await?;
        serde_json::from_slice(&b).with_context(|| format!("decoding {url}"))
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        self.check(url)?;
        let resp = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body)?)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let b = Self::read(resp, MAX_RESPONSE)
            .await
            .with_context(|| format!("POST {url}"))?;
        serde_json::from_slice(&b).with_context(|| format!("decoding {url}"))
    }
}

fn check_public(u: &reqwest::Url) -> Result<()> {
    let ip = match u.host() {
        Some(url::Host::Ipv4(v4)) => std::net::IpAddr::V4(v4),
        Some(url::Host::Ipv6(v6)) => std::net::IpAddr::V6(v6),
        _ => return Ok(()),
    };
    if !is_public(ip) {
        bail!(
            "{ip} is not a public address (set network.allow_private_peers for a private network)"
        );
    }
    Ok(())
}

/// A non-2xx response, so callers can tell "not found" from failures.
#[derive(Debug)]
pub struct HttpStatus(pub u16, pub String);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.0, self.1)
    }
}

impl std::error::Error for HttpStatus {}

/// A 429 that says how many seconds to wait.
#[derive(Debug)]
pub struct RetryAfter(pub u64);

impl std::fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rate limited: asked to wait {}s", self.0)
    }
}

impl std::error::Error for RetryAfter {}

pub fn status_of(e: &anyhow::Error) -> Option<u16> {
    e.chain()
        .find_map(|c| c.downcast_ref::<HttpStatus>())
        .map(|s| s.0)
}

/// Join a base URL and a path without doubling slashes.
pub fn join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_private_ip_literals() {
        let h = Http::new(false, false).unwrap();
        for url in [
            "http://127.0.0.1:1/v1/descriptor",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]:8481/",
            "http://10.0.0.12:8481/v1/peers",
            "file:///etc/passwd",
        ] {
            let e = h.get_bytes(url, 10).await.unwrap_err();
            let msg = format!("{e:#}");
            assert!(
                msg.contains("not a public address") || msg.contains("only http(s)"),
                "{url}: {msg}"
            );
        }
        // Host names still go through the public-only resolver.
        let e = h.get_bytes("http://localhost:1/", 10).await.unwrap_err();
        assert!(format!("{e:#}").contains("non-public"), "{e:#}");
    }
}
