# Developing Keepword

Everything is in Rust. Bug reports, ideas and changes go to the main
repository on [git.keepword.net](https://git.keepword.net/keepword/keepword).
The design, threat model and formats are in [DESIGN.md](DESIGN.md).

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
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

## Tests

Integration tests run everything over real HTTP on localhost:

- `e2e.rs`: the single-node lifecycle (capture, noise, silent and disclosed
  edits, bundles, tampering, redirects, audit, erasure)
- `network.rs`: four witnesses in four ASNs (discovery, log audits,
  cosigning, observation receipts, assigned watch requests, replacing and
  withdrawing them, dropping gossip from strangers, a disagreement nobody
  can recheck staying disputed, equivocation and forks of different
  sizes), a six-node network with one-peer gossip samples and two auditors
  per log, and a twelve-node network in two countries where rechecks
  overrule a lone dissenter and confirm, then alert on, a real regional
  difference
- `anchoring.rs`: drand beacons and Bitcoin anchoring against mock drand,
  OpenTimestamps and Esplora services
- `keepword-normalize/tests/corpus.rs`: the normalizer regression corpus
- `keepword-tlsn/tests/notary.rs`: a full MPC-TLS notarization between two
  witnesses against TLSNotary's test server
