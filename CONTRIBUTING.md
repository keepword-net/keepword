# Contributing to Keepword

Thanks for wanting to help. Keepword gets stronger with every witness, every
real-world page in the test corpus and every bug someone catches before a
hostile page or peer does.

The project lives at [git.keepword.net/keepword/keepword](https://git.keepword.net/keepword/keepword).
Registration there is open: sign up, fork, and open a pull request.

To talk a change over, or to ask anything about the code, come to
[#dev:keepword.net](https://matrix.to/#/#dev:keepword.net) on Matrix.
Everything else goes to
[#keepword:keepword.net](https://matrix.to/#/#keepword:keepword.net).
The Matrix server at keepword.net doesn't take sign-ups, so join with an
account on any other server, such as matrix.org.

## Ways to help

Not all of it is code:

- **Run a witness.** A witness in a new country or network is worth more
  than one more in a place we already cover. See
  [keepword.net/network](https://keepword.net/network/) or
  [docs/NETWORK.md](docs/NETWORK.md). Operators help each other in
  [#operators:keepword.net](https://matrix.to/#/#operators:keepword.net).
- **Grow the regression corpus.** The pages in
  `crates/keepword-normalize/tests/corpus/` are hand-built. Real pages,
  with the noise that changes between two requests to them, are the most
  valuable thing the normalizer can get (see
  [DESIGN.md §4](docs/DESIGN.md#4-normalization-v6-implemented)).
- **Teach the normalizer your language.** Relative times ("vor 3
  Stunden") and update notices ("aktualisiert", "mise à jour") are matched
  per language. If yours is missing, add it with a test. A new relative
  time changes the normalized output, so it bumps the normalizer version
  (see [below](#changes-that-need-extra-care)).
- **Report what you see.** A page that raises a silent edit when nothing
  changed, or misses one that did, makes a good issue. Include the URL and
  the output of `keepword diff`.
- **Improve the docs.** If something in the README or in `docs/` confused
  you, it will confuse the next person too.

## Before you start

Small fixes can go straight to a pull request. For anything bigger, open an
[issue](https://git.keepword.net/keepword/keepword/issues) first and say
what you plan to do. This matters most for changes to the formats, the
protocol or the normalizer (see [Changes that need extra care](#changes-that-need-extra-care)),
where a change that looks local can break witnesses already running.

[docs/DESIGN.md](docs/DESIGN.md) explains the threat model, the formats and
the network, and why they are the way they are. Read the parts your change
touches.

## Setting up

You need Rust 1.95 or newer (`rustup` or your distribution's package).
With Nix, `nix develop` (or direnv, using the `.envrc`) gives you a shell
with the toolchain, clippy and rustfmt.

```sh
git clone https://git.keepword.net/<you>/keepword.git && cd keepword
cargo build
cargo test --workspace
```

Two parts build separately:

- **Headless rendering** is behind the `render` feature
  (`cargo build --features keepword/render`) and needs Chrome or Chromium
  to run.
- **The TLSNotary tier** in `crates/keepword-tlsn` is its own workspace
  with large git dependencies. Build and test it from that directory, in
  release mode: MPC-TLS is slow unoptimized.

### Don't join the real network by accident

A node started with `keepword serve` bootstraps from the project's seed
witnesses, and from then on it is part of the public network. While you
develop, turn that off:

```sh
keepword init --asn 64500 --country DE
keepword config set network.seeds false
```

Use an ASN reserved for documentation (64496–64511; the tests use 64500)
and `.invalid` or `localhost` endpoints. The tests never dial the seeds; keep
it that way in new ones.

## Making a change

### Run the checks

CI runs these, and a pull request needs them to pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --features keepword/render
```

If you touched `crates/keepword-tlsn`:

```sh
cd crates/keepword-tlsn
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo test --release
```

If you touched `scripts/` or `packaging/`:

```sh
shellcheck -s sh scripts/*.sh scripts/ci/*.sh packaging/docker/entrypoint.sh \
    packaging/debian/build-deb.sh packaging/debian/postinst packaging/debian/prerm \
    packaging/debian/postrm packaging/debian/update-asn-db
```

CI also installs the node on several distributions and in a container.
Those jobs run on the project's runners, so you don't need to run them
yourself; check their results on your pull request.

### Write tests

New behaviour comes with a test, and a bug fix with a test that failed
before it. The integration tests in `crates/keepword/tests/` run the whole
pipeline against real HTTP servers on localhost: `e2e.rs` for a single
node, `network.rs` for several witnesses federating. Follow them rather
than mocking the network. [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)
describes the code's layout and what each test covers.

### Changes that need extra care

- **Formats and the protocol.** Attestations, logs, bundles and the peer
  API follow [Semantic Versioning](https://semver.org): they only change
  incompatibly in a new major version. Evidence made today has to verify
  years from now, and witnesses on different versions have to keep
  federating. If your change touches `keepword-core` encodings or what
  peers send each other, say in the pull request how old and new versions
  get along.
- **The normalizer.** Every attestation commits to the normalizer's
  profile, and captures are only compared within one profile. If your
  change can alter the normalized output for any input, bump `VERSION` in
  `crates/keepword-normalize/src/lib.rs`, add corpus variants for what
  should and shouldn't count as an edit, and say in the changelog what it
  means for comparing captures across the upgrade.
- **Input from pages and peers.** Everything a page or a peer sends is
  hostile until proven otherwise: sizes, nesting, timing and addresses.
  Keep the limits and the private-address guard
  (`crates/keepword-capture/src/netpolicy.rs`) intact, and add a limit
  when you add a new way in.
- **Configuration.** `docs/keepword.example.toml` is parsed by a test in
  `crates/keepword/src/config.rs`. Document a new setting there, and keep
  `keepword config set` validation in step.

## Commits and pull requests

- **Branches** are named after what they do: `feat/diff-bundles`,
  `fix/status-change-not-an-edit`, `docs/readme-run-a-witness`,
  `chore/cargo-update`.
- **Commit messages** start with a short summary in the imperative
  ("Default content retention to normalized", not "Defaulted…" or
  "feat: …"). The body says what changed and why, in plain sentences.
- **One change per pull request.** Its title is written like a commit
  summary and ends up in the merge commit.
- **Add a changelog entry** under `## [Unreleased]` in
  [CHANGELOG.md](CHANGELOG.md), in the section that fits (`Added`,
  `Changed`, `Fixed`). Lead with a bold summary, then say what a user or
  an operator notices:

  ```markdown
  ### Fixed

  - **A change of status was classified as an edit.** Going to a 403
    raised a silent-edit alert. A status change is now recorded as just
    that, and content is only compared between captures that got the
    same status.
  ```

  Docs-only and CI-only changes don't need one.
- **Version bumps and releases** are done by the maintainer. Don't change
  the version in `Cargo.toml` in a pull request.

## Reporting bugs

Open an [issue](https://git.keepword.net/keepword/keepword/issues) with:

- the Keepword version (`keepword --version`) and how you installed it
  (installer, release binary, container, from source),
- what you ran, what you expected and what happened instead,
- for capture and diff problems, the URL, or the page itself if it isn't
  public,
- for network problems, the output of `keepword net status`.

## Security vulnerabilities

Please **don't** open a public issue for a vulnerability, or bring it up in
the Matrix rooms. Email **security@keepword.net**, or send an encrypted
direct message on Matrix to
[@aelthorim:keepword.net](https://matrix.to/#/@aelthorim:keepword.net),
instead. [SECURITY.md](SECURITY.md) says what to include, what counts and
how fixes are released.

## AI-assisted contributions

Using AI tools is fine. You are the author either way: understand every
line you submit, test it, and be ready to explain and change it in review.
A pull request you can't explain may be closed.

## License

Keepword is licensed under the [GNU Affero General Public License v3.0
only](LICENSE). By contributing, you agree that your contribution is
licensed under the same terms. There is no CLA and no sign-off to add.
