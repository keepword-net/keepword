# Developing Keepword

Everything is in Rust. Bug reports, ideas and changes go to the main
repository on [git.keepword.net](https://git.keepword.net/keepword/keepword);
[CONTRIBUTING.md](../CONTRIBUTING.md) says how to send them. The design, threat model and formats are in [DESIGN.md](DESIGN.md).
Questions about the code, and changes you'd like to talk over first, go to
[#dev:keepword.net](https://matrix.to/#/#dev:keepword.net) on Matrix.

## Layout

```
crates/keepword-core       protocol: encoding, attestations, Merkle log, bundles, assignment, quorum
crates/keepword-normalize  canonicalizer, site rules, diff and silent-edit classifier
crates/keepword-capture    HTTP capture, SSRF guard, headless render, WARC export
crates/keepword-store      blob store, SQLite index, log, watchlist
crates/keepword            the `keepword` binary: CLI, peer API, federation, web UI
crates/keepword-tlsn       TLSNotary proof tier (separate workspace)
docs/                      these docs, also at keepword.net/docs
scripts/install.sh         the installer
packaging/docker/          container entrypoint and compose file
```

## Checks

Before sending a change:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --features keepword/render
```

[CONTRIBUTING.md](../CONTRIBUTING.md#run-the-checks) has the checks for
the TLSNotary tier and the scripts.

## Tests

Integration tests run everything over real HTTP on localhost:

- `e2e.rs`: the single-node lifecycle (capture, noise, silent and disclosed
  edits, bundles, tampering, redirects, audit, erasure)
- `network.rs`: four witnesses in four ASNs (discovery, log audits,
  cosigning, observation receipts, assigned watch requests, replacing and
  withdrawing them, dropping gossip from strangers, a disagreement nobody
  can recheck staying disputed, equivocation and forks of different
  sizes), a six-node network with one-peer gossip samples and two auditors
  per log, a twelve-node network in two countries where rechecks
  overrule a lone dissenter and confirm, then alert on, a real regional
  difference, and a witness at home reached through a relay
- `stability.rs`: what one hostile client or key can make a node do (CPU
  and storage stay bounded, dead peers don't starve gossip, new message
  kinds don't stall it), and peers behind a CDN placed where they are
- `keepword-capture/tests/tls.rs`: the certificate fingerprint and server
  address recorded on direct connections
- `anchoring.rs`: drand beacons and Bitcoin anchoring against mock drand,
  OpenTimestamps and Esplora services
- `keepword-normalize/tests/corpus.rs`: the normalizer regression corpus
- `scripts/noise-check.sh`: not a test, it needs the internet. It captures
  live pages twice and shows those that normalize differently, noise the
  corpus lacks: `sudo keepword watch list | scripts/noise-check.sh`
- `keepword-tlsn/tests/notary.rs`: a full MPC-TLS notarization between two
  witnesses against TLSNotary's test server
