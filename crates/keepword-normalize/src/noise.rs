//! Text-level noise: relative timestamps, live counters, invisible
//! characters, tracking parameters.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use url::{Position, Url};

static RELTIME: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // English: "5 minutes ago", "an hour ago", "3h ago"
        r"(?i)\b(\d+|an?|one)\s*(seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|wks?|w|months?|mos?|years?|yrs?|y)\s+ago\b",
        r"(?i)\bjust now\b",
        // German: "vor 5 Minuten", "vor einer Stunde"
        r"(?i)\bvor\s+(\d+|einer?|einem)\s+(Sekunden?|Sek\.?|Minuten?|Min\.?|Stunden?|Std\.?|Tagen?|Tag|Wochen?|Monaten?|Monat|Jahren?|Jahr)\b",
        r"(?i)\bgerade eben\b",
        // French: "il y a 5 minutes"
        r"(?i)\bil y a\s+(\d+|une?)\s+(secondes?|minutes?|heures?|jours?|semaines?|mois|ans?)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid regex"))
    .collect()
});

static COUNTERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(\d[\d.,]*)\s*[kKmM]?\s+(views?|comments?|likes?|shares?|reads?|readers?|followers?|Kommentare?|Aufrufe|Leser|Views)\b",
    )
    .expect("valid regex")
});

/// Unicode NFC, invisible characters removed, whitespace collapsed, and
/// time-relative or ever-changing phrases replaced by placeholders.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.nfc() {
        match c {
            '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' => {}
            c if c.is_whitespace() => space = true,
            c => {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                space = false;
                out.push(c);
            }
        }
    }
    let mut s = out;
    for re in RELTIME.iter() {
        if re.is_match(&s) {
            s = re.replace_all(&s, "<reltime>").into_owned();
        }
    }
    if COUNTERS.is_match(&s) {
        // A year is a date's, not a count: "effective from May 1, 2025 View
        // the previous notice".
        s = COUNTERS
            .replace_all(&s, |c: &regex::Captures| {
                let n = c[1].trim_end_matches(['.', ',']);
                if n.len() == 4 && (n.starts_with("19") || n.starts_with("20")) {
                    c[0].to_string()
                } else {
                    format!("<n> {}", &c[2])
                }
            })
            .into_owned();
    }
    s
}

const TRACKING_PARAMS: &[&str] = &[
    "fbclid",
    "gclid",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "_ga",
    "_gl",
    "_hsenc",
    "_hsmi",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "rb_clickid",
    "s_cid",
    "vero_id",
    "wickedid",
    "ref_src",
    "ref_url",
    "cmpid",
    "wt_mc",
    "wt_zmc",
    "at_medium",
    "at_campaign",
    // Cache busters.
    "_",
    "cb",
    "cachebust",
    "cachebuster",
    "nocache",
];

/// Parameters that only tag a site's links to itself for its analytics,
/// which it relabels at will (`duckduckgo.com/app?origin=funnel_browser`).
const SITE_TRACKING_PARAMS: &[(&str, &str)] = &[("duckduckgo.com", "origin")];

fn is_tracking(host: &str, k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    k.starts_with("utm_")
        || k.starts_with("pk_")
        || TRACKING_PARAMS.contains(&k.as_str())
        || SITE_TRACKING_PARAMS.contains(&(host, k.as_str()))
}

/// Click redirects whose path segment after a prefix is a token that
/// changes on every visit (`https://claude.ai/redirect/website.v1.<uuid>/new`).
const TOKEN_PATHS: &[(&str, &str)] = &[("claude.ai", "/redirect/")];

/// The link with a click redirect's per-visit token replaced by `<token>`.
fn mask_token(u: Url) -> String {
    for (host, prefix) in TOKEN_PATHS {
        if u.host_str() != Some(host) {
            continue;
        }
        let Some(rest) = u.path().strip_prefix(prefix) else {
            continue;
        };
        let tail = rest.find('/').map_or("", |i| &rest[i..]);
        if tail.len() < rest.len() {
            return format!(
                "{}{prefix}<token>{tail}{}",
                &u[..Position::BeforePath],
                &u[Position::AfterPath..]
            );
        }
    }
    u.into()
}

/// Resolve a link against the page URL, drop the fragment and tracking
/// parameters, and mask click redirects' per-visit tokens. Returns `None`
/// for non-web links (javascript:, mailto:, ...).
pub fn clean_link(base: &Url, href: &str) -> Option<String> {
    let mut u = base.join(href.trim()).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    // A mailto: link, behind Cloudflare's email obfuscation.
    if u.path() == "/cdn-cgi/l/email-protection" {
        return None;
    }
    u.set_fragment(None);
    if u.query().is_some() {
        let host = u.host_str().unwrap_or_default();
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !is_tracking(host, k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            u.query_pairs_mut().clear().extend_pairs(kept);
        }
    }
    Some(mask_token(u))
}

/// An address hidden by Cloudflare's email obfuscation, which serves it as
/// hex bytes XOR'd with the first, a key it picks per response, and
/// sometimes serves it in the clear instead.
pub fn cf_email(hex: &str) -> Option<String> {
    if hex.len() < 4 || !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex digits"))
        .collect();
    let (key, rest) = bytes.split_first()?;
    String::from_utf8(rest.iter().map(|b| b ^ key).collect()).ok()
}

/// Image sources are compared without their query string: CDNs put resize
/// parameters, signatures and cache busters there. `data:` URIs are replaced
/// by their hash.
pub fn clean_image(base: &Url, src: &str) -> Option<String> {
    let src = src.trim();
    if src.starts_with("data:") {
        return Some(format!(
            "data:{}",
            keepword_core::Digest::of(src.as_bytes()).short()
        ));
    }
    let mut u = base.join(src).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    u.set_fragment(None);
    u.set_query(None);
    Some(u.into())
}

const NOISE_TOKENS: &[&str] = &[
    "ad",
    "ads",
    "adslot",
    "adunit",
    "advert",
    "adverts",
    "advertisement",
    "advertising",
    "anzeige",
    "sponsor",
    "sponsored",
    "promo",
    "promoted",
    "cookie",
    "cookies",
    "consent",
    "cmp",
    "gdpr",
    "newsletter",
    "paywall",
    "outbrain",
    "taboola",
    "share",
    "sharing",
    "social",
    "related",
    "relatedposts",
    "recommended",
    "recommendations",
    "trending",
    "popular",
    "most-read",
    // User comments are not the publisher's content.
    "comment",
    "comments",
    "kommentare",
];

/// Consent-management platforms and ad networks whose ids and classes are
/// recognisable by prefix (`onetrust-banner-sdk`, `sp_message_container_123`).
const NOISE_PREFIXES: &[&str] = &[
    "onetrust",
    "ot-sdk",
    "usercentrics",
    "uc-banner",
    "cybotcookiebot",
    "cookiebot",
    "didomi",
    "sp_message",
    "sp_veil",
    "qc-cmp",
    "truste",
    "cmpbox",
    "cmpwrapper",
    "borlabs-cookie",
    "cmplz",
    "gdpr",
    "taboola",
    "outbrain",
    "google_ads",
    "disqus",
    "jp-relatedposts",
    "div-gpt-ad",
    "adsbygoogle",
];

/// Whether a class or id marks advertising, consent banners, share widgets
/// and similar per-visit noise. Matching is by whole token (so `header`
/// doesn't match `ad`), or by known vendor prefix.
pub fn is_noise_token_list(value: &str) -> bool {
    value.split_whitespace().any(|word| {
        let w = word.to_ascii_lowercase();
        NOISE_TOKENS.contains(&w.as_str())
            || NOISE_PREFIXES.iter().any(|p| w.starts_with(p))
            || w.split(['-', '_']).any(|t| NOISE_TOKENS.contains(&t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(
            clean_text("Posted 5 minutes ago by X"),
            "Posted <reltime> by X"
        );
        assert_eq!(
            clean_text("vor 3 Stunden aktualisiert"),
            "<reltime> aktualisiert"
        );
        assert_eq!(clean_text("an hour ago"), "<reltime>");
        assert_eq!(clean_text("1,234 views"), "<n> views");
        assert_eq!(clean_text("2.1K Views"), "<n> Views");
        assert_eq!(
            clean_text("effective from November 26, 2025 View the previous notice"),
            "effective from November 26, 2025 View the previous notice"
        );
        assert_eq!(clean_text("in May 2025. View all"), "in May 2025. View all");
        assert_eq!(clean_text("I went 5 miles ago"), "I went 5 miles ago");
    }

    #[test]
    fn whitespace_and_invisibles() {
        assert_eq!(clean_text("  a\u{200B}b \n\t c\u{00A0}d "), "ab c d");
        // NFC: decomposed é equals composed é.
        assert_eq!(clean_text("e\u{0301}"), clean_text("\u{00E9}"));
    }

    #[test]
    fn links() {
        let base = Url::parse("https://example.com/a/b").unwrap();
        assert_eq!(
            clean_link(&base, "../c?utm_source=x&id=5#top").unwrap(),
            "https://example.com/c?id=5"
        );
        assert_eq!(
            clean_link(&base, "/d?fbclid=1").unwrap(),
            "https://example.com/d"
        );
        assert!(clean_link(&base, "javascript:void(0)").is_none());
        assert_eq!(
            clean_image(&base, "/i.png?w=300&sig=abc").unwrap(),
            "https://example.com/i.png"
        );
        assert!(clean_link(&base, "/cdn-cgi/l/email-protection#e3908e82").is_none());
        // A site's own analytics tag, on its links only.
        assert_eq!(
            clean_link(&base, "https://duckduckgo.com/app?origin=funnel_browser").unwrap(),
            "https://duckduckgo.com/app"
        );
        assert_eq!(
            clean_link(&base, "/flights?origin=BER").unwrap(),
            "https://example.com/flights?origin=BER"
        );
    }

    #[test]
    fn click_redirect_tokens() {
        let base = Url::parse("https://www.anthropic.com/legal/privacy").unwrap();
        let link = |href: &str| clean_link(&base, href).unwrap();
        let r = "https://claude.ai/redirect/website.v1.2f2d10e1-5543-4542-a97f-e0667cb4b77e";
        assert_eq!(
            link(&format!("{r}/settings/connectors?utm_source=x&tab=1")),
            "https://claude.ai/redirect/<token>/settings/connectors?tab=1"
        );
        assert_eq!(link(r), "https://claude.ai/redirect/<token>");
        // Document IDs elsewhere stay: a link retargeted to another is news.
        for href in [
            "https://claude.ai/redirect/",
            "https://claude.ai/settings/2f2d10e1-5543-4542-a97f-e0667cb4b77e",
            "https://www.anthropic.com/legal/archive/21d66aa9-68f6-4356-ba01-2825b0f81805",
            "https://example.com/redirect/2f2d10e1-5543-4542-a97f-e0667cb4b77e/x",
        ] {
            assert_eq!(link(href), href);
        }
    }

    #[test]
    fn cloudflare_emails() {
        assert_eq!(
            cf_email("80ece1f4e1ede4f0efc0f0e1f9f0e1ecaee3efed").as_deref(),
            Some("latamdpo@paypal.com")
        );
        for bad in ["", "80", "80e", "80zz", "+0ec", "00ff"] {
            assert_eq!(cf_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn noise_tokens() {
        assert!(is_noise_token_list("sidebar ad-slot"));
        assert!(is_noise_token_list("cookie_banner"));
        assert!(is_noise_token_list("most-read"));
        assert!(!is_noise_token_list("header shadow"));
        assert!(!is_noise_token_list("article-body"));
        assert!(is_noise_token_list("onetrust-consent-sdk"));
        assert!(is_noise_token_list("sp_message_container_893412"));
        assert!(is_noise_token_list("usercentrics-root"));
        assert!(is_noise_token_list("CybotCookiebotDialog"));
        assert!(!is_noise_token_list("adress")); // German spelling, not "ad"
    }
}
