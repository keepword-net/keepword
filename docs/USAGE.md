# Using Keepword

The `keepword` command runs a node, and checks proofs without one. To set a
node up as a service, see [INSTALL.md](INSTALL.md); to start a network or
join one, see [NETWORK.md](NETWORK.md).

## Quick start (from source)

```sh
cargo build --release
alias keepword=./target/release/keepword

keepword init --asn 3320 --country DE        # creates ./keepword-data
keepword capture https://example.org/privacy
keepword watch add https://example.org/privacy --every 6h
keepword serve --watch                        # web UI on http://127.0.0.1:8480
```

When the page changes:

```
$ keepword capture https://example.org/privacy
attestation 7c1e…
  ...
CHANGED since 3f2a90c1b7d4: silent edit: +1 -1 lines

--- before
+++ after
@@ -3,4 +3,4 @@
 h1: Privacy policy
-p: We never sell your data.
+p: We may share your data with partners.
```

Proving it to someone else:

```sh
keepword export https://example.org/privacy --at 2026-05-01 -o evidence.json
# they run, with no node and no network:
keepword verify --bundle evidence.json
```

```
  [ok  ] signature      signed by witness 7b757b5c…
  [ok  ] tree head      size 1204 root 3fc10f581373
  [ok  ] log inclusion  leaf 1187 of 1204
  [skip] cosignatures   no other witness has cosigned this tree head
  [ok  ] not before     captured after 2026-05-01T09:14:03Z (drand round 28274493, 2s before the claimed time)
  [ok  ] headers        467 bytes, hash 13d3817dd54e
  [skip] body           not included
  [ok  ] normalized     matches 61a162ac8062
  [skip] renormalize    needs the body and a norm commitment

VERIFIED: LOGGED

This bundle shows
  - a witness (7b757b5c…) signed that it saw this page at the stated time
  - the statement is in that witness's append-only public log
  ...
```

That is a node on its own, keeping the default `content.retain =
normalized`. With `full`, the bundle carries the page's bytes, and `body`
and `renormalize` are checked too. In a network, other witnesses cosign the
log (`LOGGED + COSIGNED`), and `verify --esplora URL` checks the Bitcoin
anchor.

## Commands

| Command | What it does |
|---|---|
| `init [--asn N] [--country CC] [--retain full\|normalized\|none]` | Create key, config and empty log |
| `id` | Show witness key and log head |
| `capture URL [--render]` | Fetch now, attest, log, compare with the previous capture |
| `verify URL\|ID [--at TIME]` / `verify --bundle FILE` `[--esplora URL]` | Run every check on a stored capture or a bundle; `--esplora` checks Bitcoin anchors |
| `export URL\|ID [--at TIME] [--no-content] [-o FILE]` | Write an evidence bundle |
| `warc URL\|ID [--at TIME] -o FILE` | Rebuild a WARC 1.1 file for a capture |
| `history URL` | All captures and detected changes |
| `diff URL` / `diff ID ID` / `diff --bundle FILE FILE` | Diff the last two versions, two captures, or two bundles (no node needed) |
| `watch add\|rm\|list\|run [--once]` | Manage and run the watchlist |
| `log head\|consistency OLD [NEW]\|audit` | Tree head, consistency proofs, full self-audit |
| `purge URL [--forget]` | Erase stored content (and records); the log stays valid |
| `serve [--addr A] [--api-addr B] [--watch] [--anchor]` | Web UI (private), peer API (public), scheduler, federation and anchoring loops |
| `net add-peer URL\|remove-peer KEY\|peers\|sync\|status\|lookup IP` | Federation with other witnesses; `status` warns about misconfiguration |
| `request URL [--every 1h] [--for 7days] [--render]` / `request URL --cancel` | Ask the network to watch a URL (asking again replaces the request), or withdraw it |
| `requests [--mine]` | Active network watch requests |
| `verdict URL` | What independent witnesses agree the URL served |
| `alerts [--limit 50]` | Split, silent-edit and equivocation alerts |
| `anchor submit\|upgrade\|list\|export` | Bitcoin anchoring via OpenTimestamps |
| `beacon [ROUND]` | Fetch and verify a drand beacon |
| `config show\|get\|set\|unset` | Read or change `keepword.toml`, with validation |

`--at` takes RFC 3339 or `YYYY-MM-DD` (end of that day, UTC). IDs can be
abbreviated. `--json` gives machine-readable output.

The node's data directory is `--dir` or `KEEPWORD_DIR` if given, else
`./keepword-data` if it exists, else the node the installer set up
(`/var/lib/keepword`). Run as root, `keepword` switches to the owner of that
directory, the service user, before doing anything, so `sudo keepword …`
just works on an installed node.

## Configuration

`keepword-data/keepword.toml`; see [keepword.example.toml](keepword.example.toml).
The important settings:

- `content.retain`: what to keep. `normalized` (the default: diffs without
  raw bytes), `full` (raw bytes too) or `none` (hashes only). Attestations
  and the log never contain page content.
- `[[rules]]`: per-site normalization (extra elements to drop, content
  root). Rules are part of the normalizer profile each attestation commits
  to.
- `capture.use_system_proxy`: off by default. A proxy changes your vantage
  point, and a TLS-intercepting one hides the server's certificate.

## Running in a network

The installer sets this up for you. By hand:

```sh
keepword init --asn 3320 --country DE
# in keepword-data/keepword.toml:
#   [network]  endpoint = "https://witness.example.org"   peers = ["https://other.example"]
#   [quorum]   asn_db = "/var/lib/keepword/ip2asn-combined.tsv"   (from iptoasn.com)
keepword serve --api-addr 0.0.0.0:8481 --watch --anchor
```

Put the API (`/v1/...`) behind TLS on your public endpoint. Keep the web UI
(`--addr`, default `127.0.0.1:8480`) private, because it shows page content.
Peers corroborate each other's location from the addresses they see pushes
come from. Verdicts only count witnesses whose ASN is corroborated this
way, so every node needs an IP-to-ASN table.

## Headless rendering

For pages that only exist after JavaScript runs:

```sh
cargo build --release --features render
keepword capture --render https://example.org/app
```

This needs Chrome or Chromium; set `capture.chrome` if it isn't on `PATH`.
The browser resolves hosts itself, so the private-address guard only covers
the page's own address, not its sub-resources. Only render URLs you chose;
network requests are rendered only with `network.render_requests`.

## TLSNotary proof tier

For high-value pages, a second witness can notarize the TLS session itself
(MPC-TLS via [TLSNotary](https://tlsnotary.org)). Then even the capturing
witness couldn't have made the content up on its own:

```sh
cd crates/keepword-tlsn          # separate workspace, Rust 1.95+
cargo build --release
keepword-tlsn serve --addr 0.0.0.0:8482                    # on the notary
keepword-tlsn capture https://example.org/terms --verifier notary.example.net:8482 --verifier-key <hex>
```

Bundles then carry the notary's receipt and the transcript, and
`keepword verify` checks both.
