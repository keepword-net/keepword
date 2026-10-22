# Changelog

All notable changes to Keepword, called Witness before 2.0.0. Versions
follow [Semantic Versioning](https://semver.org): from 1.0.0 on, the
evidence formats (attestations, logs, bundles) and the network protocol
only change incompatibly in a new major version.

## [Unreleased]

### Added

- **A web UI that runs a node without the CLI**, for people who won't
  open a terminal, as on Umbrel or TrueNAS. The overview lists the
  changes to review, each with the words that changed marked, says how
  the node is doing, and what to do first on a new node. Pages can be
  added several at once, also asking the network to watch them, and each
  has its checks, changes, what other witnesses saw, its evidence, and a
  way to delete its content. A change reads like the page, with the
  whole page and the normalized lines a tab away, and is marked
  reviewed. The network page asks the network to watch pages, adds and
  forgets witnesses and syncs. Settings has the common ones as a form,
  all of `keepword.toml` for the rest, the password, and a restart.
  The CLI does all of this as before.

### Changed

- **Normalized text is `keepword-norm/5`**, for the fixes below. Captures
  only compare when they were normalized the same way: until every
  witness upgrades, verdicts and rechecks draw on fewer witnesses, and a
  witness doesn't compare a URL's first capture after the upgrade with
  the one before. Upgrade every witness.
- **The web UI asks for a password when it listens beyond localhost**,
  as the container's does: the first visit sets it, or
  `KEEPWORD_UI_PASSWORD`. On localhost it asks for none, unless one is
  set, and only answers requests addressed to localhost, so no website
  can reach it through a browser. `keepword serve --ui-auth none`
  (`KEEPWORD_UI_AUTH`) leaves the login to a proxy in front of it.
  Forms only work from the UI's own pages.
- **`keepword serve` exits with status 75 when the web UI restarts it**
  to apply new settings, for systemd (`Restart=on-failure`) or the
  container runtime to start it again. The UI only offers to where one
  of them runs it, or `KEEPWORD_SUPERVISED=1` says another does; else it
  says to restart Keepword by hand.
- **The store's schema is version 6**: changes record when they were
  reviewed. Once this version has opened a node's data, 2.4.0 refuses
  it.

### Fixed

- **A site's menus, banners and footers raised silent-edit alerts.** On a
  page without `<main>`, like Criteo's and Cloudflare's policies, the
  whole body was compared, so a rotated story in the menu or a renamed
  footer link showed as a silent edit on every page of the site. Site
  chrome is now dropped: `<nav>`, the ARIA `navigation`, `banner` and
  `contentinfo` landmarks, and a `<header>` or `<footer>` outside any
  section. A section's own header stays. Chrome in a plain `<div>` still
  needs a site rule.
- **Adding a canonical link raised a silent-edit alert.** The
  `canonical:` line is gone from the normalized text: it is SEO markup a
  site changes on all its pages at once, not content.
- **A peer looked fine as long as its last sync had worked, however long
  ago.** A round syncs a sample of the peers, so one that went down
  wasn't noticed until it was picked again, and if the node stopped
  syncing, every peer kept looking as it did then. A peer is now in sync
  only when a sync worked within the hour, and quiet after that;
  `keepword net peers` says when it last worked rather than when it was
  last tried, and `net status` counts the peers in sync and warns when
  none is. `net status` also says federation is off when no endpoint,
  relay or peers are set, as `keepword serve` then doesn't sync, even
  with peers it remembers.

## [2.4.0] - 2026-10-20

Witnesses at home, reached through relays, witnesses that need no
domain, a container image with each release, and diffs and alerts that
read easier. Formats, the normalizer and the protocol are unchanged:
2.3.0 and 2.4.0 witnesses federate, compare captures and verify each
other's bundles as before. Relaying is new, and only a witness at home
and the witness relaying for it need 2.4.0: with `--relay auto`, a
witness at home finds one among those it knows that run it. Upgrade the
witnesses that accept connections first, so there are relays to find.

### Added

- **Witnesses at home.** A node that can't accept connections, behind
  NAT, joins with `--relay auto` (`network.relay`, `KEEPWORD_RELAY`):
  it finds a witness that relays for it among those it knows, and peers
  reach its API at `<relay>/v1/relay/<key>`, through polls it keeps open
  to that witness. It is audited, assigned requests and counted like any
  witness, and located in its home network. Observing nobody itself, it
  places witnesses where its relay does (`/v1/candidates`). A relay
  stores nothing and can't forge, only withhold. Every witness relays,
  for up to `network.relay_witnesses` others (8, at most 2 per address).
  Only the witness and its relay need this version (DESIGN.md §6.9).
- **A container image with each release**, for x86_64 and arm64:
  `git.keepword.net/keepword/keepword:2`. It holds the release's static
  binary, so a NAS box no longer compiles Keepword to run it.
  `packaging/docker/compose.yaml` pulls it, and builds the checkout with
  `--build`.

### Changed

- **A witness needs no domain.** With `--public-api` and no `--endpoint`,
  the installer advertises `http://<public IP>:<port>`. Before, it
  advertised nothing, so the witness was never assigned requests. Peers
  always accepted plain HTTP: everything they fetch is signed.
- **`keepword diff` and `capture` show a change compactly on a
  terminal**: the changed lines with a line around them, long ones cut
  short, and the words that changed highlighted. Piped, or with
  `NO_COLOR`, they print the unified diff as before.
- **`keepword alerts` reads easier**: kinds in words, times to the
  minute, an alert repeated in a row as one line (`×16 since …`), and who
  raised it, this node or a peer's host.
- **Building needs Rust 1.95**, as the TLSNotary tier already did (was
  1.90). The installer used a system Rust from 1.85 on, which then failed
  to build; it now uses one from 1.95 on and otherwise installs stable.

### Fixed

- **`keepword diff URL` showed a silent edit on pages that hadn't
  changed**, after the upgrade to 2.3.0. It diffed the latest capture
  with the last one made by the previous normalizer, which differs at
  least in its version line, and `keepword history` counted it as a new
  version. Both now only compare captures normalized the same way, as
  change detection always did, and `diff` warns when two captures it was
  given weren't.
- **`keepword purge` kept the TLS transcripts of notarized captures**,
  each the whole response, with `content.retain = "full"` and the
  TLSNotary tier. They are deleted with the rest, and `--forget` drops
  the receipts too.
- **Anchoring asked the calendars again for every anchor not yet
  confirmed, every round**, also for proofs they had completed. Without
  a block explorer (`anchor.esplora_url = ""`) none is ever confirmed,
  so the requests grew with every anchor made. A calendar is no longer
  asked once it has given its Bitcoin proof.
- **`keepword verify --esplora` gave up at the first block the explorer
  couldn't tell about**, say one not indexed yet, and left the anchor
  unchecked. It tries the other claimed blocks.

## [2.3.0] - 2026-10-19

A second default seed, a normalizer that no longer raises silent-edit
alerts for two kinds of per-request noise, a live check for more of it,
and `verify --esplora` settling the anchor check. Formats and the protocol
are unchanged: 2.2.0 and 2.3.0 witnesses federate and verify each other's
bundles, all but re-running the other version's normalizer. The normalizer
is new, though, and captures only compare when they were normalized the
same way: until every witness runs 2.3.0, verdicts and rechecks draw on
fewer witnesses, and a witness doesn't compare a URL's first capture after
the upgrade with the one before. Upgrade every witness.

### Added

- **`scripts/noise-check.sh`** captures live pages twice and shows those
  that normalize differently, noise the normalizer misses:
  `sudo keepword watch list | scripts/noise-check.sh`.

### Changed

- **`https://seed2.keepword.net` is a second default seed**, next to
  `https://seed1.keepword.net`.
- **Dependencies are updated to their latest versions**, among them
  reqwest 0.13, rusqlite 0.40, scraper 0.27 and ed25519-dalek 3. HTTPS
  uses aws-lc-rs instead of ring and still checks certificates against
  the system's roots. The normalizer's output is unchanged on the test
  corpus.
- **The container image is built on Debian 13 (trixie)** instead of
  Debian 12 (bookworm).
- **Normalized text is `keepword-norm/4`**, for the two normalizer fixes
  below. As every normalized text starts with the version, every
  normalized hash changes.

### Fixed

- **A link with a per-visit token raised a silent-edit alert on every
  capture.** Anthropic's policies link to claude.ai through
  `https://claude.ai/redirect/website.v1.<uuid>/…`, with a new UUID for
  each request. Known click redirects like this one have the token
  replaced by `<token>`. Other UUIDs in links are document IDs and stay.
- **A page behind Cloudflare changed back and forth when only some
  responses obfuscated its email addresses.** Addresses hidden as
  `[email protected]` are decoded, and the obfuscated `mailto:` links
  are dropped like the plain ones.
- **A page served gzipped normalized to garbage.** Keepword's requests
  accept no encoding, which allows any, and Amazon gzips some responses
  anyway, so each capture differed from the last. A gzipped text body is
  inflated before normalizing, up to 32 MiB; it is still hashed as served.
- **`keepword verify --esplora` left the anchor unchecked.** The chain's
  answer went into a separate `bitcoin` line, while the `anchor` line
  stayed skipped and the summary said there was no checked anchor. The
  chain now settles the `anchor` check itself: it passes with the block
  the tree head existed by, or fails if no claimed block has the
  committed Merkle root. In `--json` output the `bitcoin` check is gone.
  A block that two calendars confirmed in is listed and checked once,
  earliest block first, by `verify` and by `anchor upgrade`.

## [2.2.0] - 2026-10-15

A one-line install from the release binary, diffs between two bundle files,
normalized retention by default for new nodes, and status changes no longer
counted as edits. Formats, the normalizer and the protocol are unchanged:
2.1.0 and 2.2.0 witnesses federate and compare captures as before.

### Added

- **`keepword diff --bundle A B`** diffs two bundle files with no node or
  store, so whoever was sent two bundles can see what changed. Both must
  verify and carry their normalized text.
- **`install.sh --binary`** installs the latest release's static binary
  instead of building, so small machines need no compiler or Rust, and the
  installer runs piped: `curl -fsSL …/scripts/install.sh | sudo sh -s --
  --binary`. A truncated download runs nothing, and the confirmation is
  asked on the terminal.

### Changed

- **New nodes keep normalized text, not raw bodies, by default.**
  `content.retain`, `keepword init --retain`, the installer and the Docker
  image now default to `normalized`: diffs work as before, with no raw
  copy of third-party pages on disk. Pass `full` to keep raw bodies too.
  Existing nodes keep their setting.

### Fixed

- **A change of status was classified as an edit.** A blip of errors had
  the error page diffed against the page: going to a 403 raised a
  silent-edit alert, and coming back was a "disclosed edit" if the page
  had an update notice. A status change is now recorded as just that, and
  content is only compared between captures that got the same status;
  back from an error, with the last capture that succeeded the same way,
  so an edit can't hide behind a blip. The web UI's and `keepword diff`'s
  summaries follow suit.

## [2.1.0] - 2026-10-10

Fixes from an end-to-end bug hunt, most of them for what a hostile page or
peer could make a witness do, and a logo. 2.0.0 and 2.1.0 witnesses
federate and verify each other's bundles, all but re-running the other
version's normalizer. The normalizer is new, though, and captures only
compare when they were normalized the same way: until every witness runs
2.1.0, verdicts and rechecks draw on fewer witnesses, and a witness
doesn't compare a URL's first capture after the upgrade with the one
before. A TLSNotary session needs both witnesses on 2.1.0. Upgrade every
witness.

### Added

- **A logo:** a gold seal stamped with a quotation mark, for what a page
  said, sealed. `docs/assets` has the seal on its own (`logo.svg`, and
  `logo-512.png` for avatars) and with the name, for light and dark
  backgrounds (`logo-wordmark.svg`, `logo-wordmark-white.svg`). The
  banner and the social preview use it (`social-preview.svg` is the
  source of `social-preview.png`), and the web UI shows it in its header
  and as its tab icon.

### Changed

- **The project moved to git.keepword.net/keepword/keepword**, a Forgejo
  instance, and its CI to Forgejo Actions. The fetcher's user agent links
  there, and the default seed is `https://seed1.keepword.net`.
- **Normalized text is `keepword-norm/3`.** The normalizer has limits for
  hostile pages now, a link's text leaves out the links nested in it and
  only HTML `<a>` elements are links (see Fixed), and where tags are
  misnested, a link's text no longer runs on past the link or stops
  short. Such pages normalize differently, and as every normalized text
  starts with the version, every normalized hash changes.

### Fixed

- **A hostile page could crash a witness or keep it busy for days.** The
  normalizer read pages recursively, so about 25 kB of nested tags
  overflowed its stack and aborted the node. The HTML parser took time in
  the square of how deep a page nests (minutes for a megabyte, days for
  the 32 MiB a capture may have), formatting tags it reopens in every
  paragraph made 160 kB of markup take 3.6 GB, and a link nested in links
  repeated the text of every link inside it: a 1 MB page normalized to
  4 GB. Pages are now read without recursion, and tags nested more than
  512 deep are ignored, their text kept; in SVG and MathML, `<link>`,
  `<style>` and the like open elements too, and there they close at once.
  A page's tree stops growing at one node per two bytes, a link's text
  leaves out the links inside it, and only HTML `<a>` elements are links:
  an SVG or MathML `<main>` can be where the normalizer starts, and `<a>`
  nests freely there. A page built for it still takes about a minute.
- **Diffs of big rewrites could run for days.** Finding the fewest changes
  between two versions takes time in the square of the lines changed, two
  minutes for 100 000. After a second, the rest now shows as replaced,
  and a line a diff shows as both removed and added, such as an update
  notice the page already had, no longer makes an edit disclosed.
- **Gossip from newer versions could stop a node syncing with a peer.**
  Pulls skip message kinds a node doesn't know, but a page of nothing
  else didn't move the cursor past them, so the node asked for that page
  forever. Pushes carrying them failed authentication, so their sender got
  no observation receipt. Pulls now move past every message, and pushes
  list the IDs of the messages they carry (2.0.0 witnesses ignore them).
- **Watch requests for the longest intervals were due every round.** The
  network takes requests for any interval from ten minutes up. Past
  292 million years, the interval wrapped around in the scheduler, and
  every witness assigned captured the URL every 15 seconds (debug builds
  panicked). Such a watch is now never due again.
- **`keepword log audit` panicked** when leaves were missing under a tree
  head, which is what it is there to find, and took time in the square of
  the log's size. It also ended with "VERIFICATION FAILED" when every
  check passed; it now says whether the audit passed.
- **One bad calendar or explorer answer stopped anchor upgrades.** A
  calendar answering anything but a timestamp failed every upgrade, and a
  failed block lookup discarded the proofs calendars had just completed.
  Each calendar or explorer that fails is now logged, once a round, and
  the rest go ahead.
- **`keepword request` withdrew the old request before checking the new
  one,** so a replacement the network refuses, such as `--every 1m`, left
  the URL with no request at all.
- **`keepword init` wrote configs it can't load,** for instance with
  `--country DEU`. No command worked after that, not even `keepword config`
  or `keepword init`. It now refuses them and writes nothing.
- **`keepword config unset` didn't turn off drand or the block
  explorer.** Left out of the file, `beacon.drand_url` and
  `anchor.esplora_url` loaded as their defaults again, so the node kept
  using api.drand.sh and blockstream.info, and tests meant to run offline
  fetched drand beacons. Unset, they are now saved as "", which turns
  them off. One unset with 2.0.0 is still missing from the file: unset it
  again.
- **A failed round of watches stopped `keepword serve`,** API and gossip
  included, for instance when the database stayed busy for ten seconds.
  It is now logged and retried, like a failed sync.
- **A capture that couldn't be compared with the one before was
  "unchanged"** in `keepword capture` and the watch log, after new site
  rules for instance. They now say it wasn't compared.
- **A bundle's TLSNotary receipt could be dated 292 million years from the
  capture** and pass the check that both happened within ten minutes: the
  subtraction wrapped around. Debug builds panicked on such bundles, and on
  pushes sent at such times.
- **A TLSNotary session could hang for good.** One in six stalled
  locally while the two witnesses set up MPC-TLS, and one CI run in
  twenty. The stall was inside TLSNotary, which has since fixed its stream
  multiplexer and replaced its executor; keepword-tlsn now builds against
  that revision, where none of 800 sessions hung. Sessions there share
  one thread pool, where each used to start a thread per CPU core.
  The revision speaks a newer protocol: a 2.0.0 prover hangs on a 2.1.0
  verifier, and a 2.1.0 prover gets an error from a 2.0.0 one.
- **Notarization had no timeouts.** A verifier that went quiet left
  `keepword-tlsn capture` waiting forever, and a prover that went quiet,
  or never sent its hello, held a verifier's connection for as long as it
  liked. Either side now gives up on a session after ten minutes, and on a
  hello or a receipt lookup after 30 seconds. A session that fails or is
  refused now closes its connection, and a refused prover still hears why.
- **A flood of connections stopped `keepword-tlsn serve`.** Once it ran out
  of file descriptors, at about a thousand open connections with the usual
  limit of 1 024, accepting failed and the service exited. It now logs the
  error and tries again a second later, and idle connections close within
  30 seconds.

## [2.0.0] - 2026-09-30

**Witness is now Keepword.** "Witness" is a crowded name, too hard to find
and to tell apart. Keepword turns the problem this project is about, web
pages that don't keep their word, into a promise. "Witness" stays as the
role: you still run a witness, witnesses still testify, and "Keepword is a
network of independent witnesses".

The whole project changes name at once, protocol included, so this is a
clean break: 2.0.0 nodes don't federate with 1.x nodes, and 1.x logs and
bundles don't verify with 2.0.0. Nothing carries over from a 1.x node.

### Changed

- **The command is `keepword`** (and `keepword-tlsn` for the TLSNotary
  tier), and the crates are `keepword` (the node), `keepword-core`,
  `keepword-normalize`, `keepword-capture`, `keepword-store` and
  `keepword-tlsn`. The repository is github.com/keepword-net/keepword.
- **Names on a node:** the data directory is `/var/lib/keepword` (or
  `./keepword-data` for a development node, or `KEEPWORD_DIR`), the config
  file `keepword.toml`, the service user `keepword`, the services
  `keepword.service`, `keepword-asn-update.timer` and
  `keepword-notary.service`, and the installer also uses `/opt/keepword`,
  `/usr/local/lib/keepword`, `/etc/keepword/data-dir` and
  `/etc/caddy/keepword.caddy`. The key file is still `witness.key`: it is
  the witness's identity.
- **Container:** the image is `keepword`, with a `keepword-data` volume, and
  it is configured with `KEEPWORD_ASN`, `KEEPWORD_COUNTRY`,
  `KEEPWORD_RETAIN`, `KEEPWORD_ENDPOINT`, `KEEPWORD_PEERS`,
  `KEEPWORD_SEEDS`, `KEEPWORD_BEHIND_PROXY`, `KEEPWORD_CLIENT_IP_HEADER`
  and `KEEPWORD_ASN_DB`.
- **Protocol and evidence formats (incompatible):** every signing domain
  and hash tag starts with `keepword` instead of `witness`
  (`keepword/attestation/v2`, `keepword/tree-head/v1`,
  `keepword url-key v1`, …), evidence bundles are `keepword-bundle/1`,
  normalized text starts with `keepword-norm/2`, so normalized hashes
  change too, and a WARC export's attestation record is
  `application/vnd.keepword.attestation+json`.
- The default User-Agent is `Mozilla/5.0 (compatible; Keepword/2.0.0;
  +https://github.com/keepword-net/keepword)`, and peers see `keepword/2.0.0`.
- Release archives are `keepword-vX.Y.Z-TARGET.tar.gz`. The web UI, the
  banner and the social preview say Keepword.

### Upgrading from Witness 1.x

Start every node over. On each one, remove Witness with its own installer
(`sudo sh scripts/install.sh --uninstall --purge` in a 1.x checkout), then
install Keepword. Upgrade the default seed first: new nodes join through
it.

## [1.2.0] - 2026-09-29

Witnesses behind Cloudflare or another CDN, and CI kept off the public
network. Nothing changes in the protocol or the stored data; 1.1.x and
1.2.0 witnesses work together. A witness behind a CDN places every peer in
the CDN's network until its proxy passes on client addresses: see
`network.client_ip_header` below and docs/INSTALL.md.

### Added

- **`network.client_ip_header`: witnesses behind Cloudflare or another
  CDN.** Behind a CDN, the address a witness's reverse proxy sees is the
  CDN's. The proxy can work out the client's address and pass it on in a
  header of its own, e.g. Caddy's `header_up X-Real-IP {client_ip}`; the
  node now takes the address from that header. Container:
  `WITNESS_CLIENT_IP_HEADER`.
- `witness net status` warns when peers seem to connect from Cloudflare.
- The installer's `--no-seeds` and the container's `WITNESS_SEEDS=0` keep a
  node off the public network, for separate networks and tests.

### Fixed

- **Behind a CDN, every peer was placed in the CDN's network.** The node
  took the address its proxy added to X-Forwarded-For, which behind a CDN
  is the CDN's edge. It signed that as every peer's location (AS13335
  behind Cloudflare), told each peer it was there, and nothing warned.
- **CI runs joined the public network.** The installer and container tests
  started nodes with the default seeds, and each run left a few dead
  witnesses in the seed's peer table.
- **Behind a proxy that adds its own X-Forwarded-For line**, such as
  HAProxy, the node read the first line, which the client wrote. It now
  reads the last.
- With the API listening on `[::]`, which takes IPv4 connections too,
  every IPv4 client shared one rate-limit bucket.

## [1.1.1] - 2026-09-28

Security, correctness and stability fixes: location corroboration,
rechecks and fork checks, and what one hostile client or key can make a
witness do. Nothing changes in the protocol or the stored data; upgrading
is recommended for every witness before a network goes public.

### Fixed

- **Two servers could place any number of keys in any networks.**
  Receipts counted once per observer network, but two observers on two
  real networks could sign receipts putting keys that never connected to
  a node in as many networks as they liked, and assignment, verdicts and
  recheck draws counted each. A node now places another witness only
  where it saw that witness connect itself; receipts only tell a node
  where it is itself. New witnesses count at every node once they have
  pushed to it, within about a day.
- **Anyone could spend other witnesses' recheck captures.** A recheck
  request only had to come from a key assigned to the URL, and any key is
  assigned to some URLs (it only has to try enough of them). Drawn
  witnesses now fetch the assigned witnesses' attestations first and only
  capture when they see the round disputed themselves; skipped requests
  don't use up `quorum.max_rechecks_per_hour`.
- **Honest witnesses were charged for page edits.** A witness that
  captured a page just before its publisher edited it failed the rechecks
  made after the edit, and five of those in a week left its network out
  of verdicts. A version other networks saw too, captured before any other
  network saw the version the rechecks confirmed, now costs nothing.
- **Recheck draws were judged by the wrong round.** A draw that completed
  after a newer round had started was settled against the newer round's
  versions, so witnesses could be charged for reporting the current page.
  Each draw is now judged by the versions of the round it was drawn for,
  and settled even when that round is no longer the current one; only
  draws of rounds with the same versions add up in a verdict.
- **A forked log could hide behind its own old heads.** Auditors checked
  gossiped heads oldest first, eight per audit, so a log could sign heads
  of the history its forks share (all consistent) and gossip them ahead of
  the ones that exposed it. The biggest heads are now checked first.
- **Anyone could keep a witness's CPU busy with fake drand beacons.**
  Beacons are accepted from anyone and each check is a BLS pairing
  (about 2.5 ms): one push of 1000 garbage beacons cost 2.5 s of CPU, and
  600 pushes a minute from one address kept about 26 cores busy. A node
  now only checks beacons it can use, at most 20 at once and one every
  three seconds after that, before any signature is checked.
- **One key could fill every node's disk.** Any key gets into the peer
  table with a descriptor and could then push unlimited alerts, tree heads
  or receipts, which every node stored and forwarded. A node now takes an
  hour's worth of each kind from one key at most (10 to 200 messages), and
  2000 an hour in all from keys it has never synced with nor seen push to
  it. Alerts dated ahead, which were never pruned, are refused.
- **Dead peers could crowd working ones out of gossip.** Each round synced
  a random sample of peers, so keys announcing endpoints nobody answers on
  could fill it. Three quarters of each round now go to peers that synced
  within a day.
- **A watch request could be made never to expire.** Its duration was
  checked by subtracting two times that wrap around in release builds; a
  request from 146 million years ago that expires as long from now passed.
- **Rechecks counted captures made for later draws.** Successive draws of
  a URL overlap, and a witness drawn for two of them had its later
  capture, of a page maybe edited since, count for both. Each draw now
  counts each witness's first capture after the round it rechecks.
- **Work that grew with the square of the network.** Locating the peers
  checked every stored receipt about each one (450 ms per lookup at 200
  peers, and it ran per followed URL); every incoming watch request
  recomputed every request's assignment; followed URLs re-downloaded a week
  of attestations from every assigned witness every ten minutes; and
  receipts flooded, one per pair of witnesses. Now: one receipt per peer,
  assignments once per sync, only new attestations after the first fetch,
  and receipts go only to the witness they are about.
- **A failing step stopped the rest of a sync round**, pruning included,
  and one URL that couldn't be evaluated stopped the review of all others.
- **Verdicts counted every key in an epoch whose beacon was missing.**
  Without the beacon nobody can tell who was assigned, so no attestation
  from that epoch counts now.
- **Verdicts dropped captures of witnesses whose clock is a little
  behind.** They allowed no skew between a capture and its drand beacon,
  while bundles allow a minute; both allow a minute now.
- **X-Forwarded-For was taken from any client** with
  `network.trust_forwarded_for` on, say through a published container
  port, and where a peer connects from now decides where it is. The header
  is only taken from connections from this machine or a private network.

## [1.1.0] - 2026-09-28

Disagreements are now settled by reproduction instead of by vote. Upgrade
every witness; 1.0.x witnesses keep working alongside 1.1.0 ones but don't
take part in rechecks.

### Added

- **Rechecks.** When the witnesses assigned to a URL disagree, the round
  is *disputed*, and witnesses drawn at random from the reporters'
  countries, one per network and none of them assigned, capture the page
  again. Only versions they reproduce count: a real regional difference is
  confirmed as a split, and a made-up one is overruled. The draw uses
  drand and a fixed slot of time, so nobody can pick or retry it. New
  verdict: `DISPUTED`, while rechecks run or when nothing could be
  confirmed. See DESIGN.md §6.5.
- **Split alerts need repeated confirmation**: 3 of the last 4 settled
  recheck rounds for the URL (`quorum.split_confirmations`,
  `quorum.split_rounds`).
- **Failed claims.** A version the rechecks could have reproduced and
  didn't counts against the network address it came from; after 5 in a
  week (`quorum.max_failed_claims`) that network is left out of verdicts.
- **Forks of different sizes are detected.** Auditors check the heads
  other auditors report for a log against the one they verified; a log
  that can't prove them consistent is excluded and alerted on.
- **Rate limit on the public API**: 600 requests a minute per address
  (`network.api_requests_per_minute`), answered with HTTP 429 beyond that.

### Changed

- Only the witnesses assigned to a URL count in its verdict.
- Log proofs, bundles and new tree heads cost O(log n) instead of a pass
  over the whole log, so large logs stay fast and the proof endpoints
  can't be used to exhaust a node.
- Verdict JSON has a `groups` list (every version compared) and a
  `rechecks` count.

### Known limitations

- A difference seen only from one country is only confirmed if that
  country has at least 3 witnesses on different networks beyond the ones
  assigned. Small networks show `DISPUTED` where 1.0 showed `SPLIT`.
- A country where one operator runs most witnesses decides what is seen
  from there, and a round whose assigned witnesses are all one operator's
  agrees on whatever they say.

## [1.0.1] - 2026-09-28

Security fixes. Upgrading is recommended for every witness; nothing
changes in the protocol or the stored data.

### Fixed

- **Sybil keys could invent networks.** Location corroboration counted
  observers by key, and keys are free: one server with 20 keys could
  vouch that each of its keys sat in a different network, and verdicts
  and assignment then counted 20 independent witnesses. A node now
  trusts where it saw a witness connect itself, and otherwise counts only
  observers it has seen connect, once per network.
- **One witness could blind a verdict.** The comparison window was
  anchored at the newest attestation, so a single attestation dated in
  the future pushed every honest one out of it. Future-dated attestations
  and ones older than their own drand beacon are now rejected, and the
  window compared is the one covering the most independent networks.
- **Extra keys could crowd out honest witnesses.** When captures used
  different methods or normalizer profiles, the class with the most keys
  was compared. It is now the class with the most networks.
- **`witness verify` claimed more than it proved.** It printed VERIFIED
  whenever no check failed, even for a bare signed attestation. It now
  grades the result (SIGNED ONLY, LOGGED, LOGGED + COSIGNED) and says in
  plain words what the bundle shows and what it doesn't, in the CLI, the
  JSON output and the web UI. Cosignatures are reported as "other keys",
  since a bundle alone can't show whose keys they are.

### Known limitations

- A log that shows different auditors histories of *different* sizes is
  not yet detected; only same-size forks are. The 1.0.0 notes overstated
  this.
- A witness that really connects from many networks (rented servers or
  proxies) still counts as many. Corroborated location shows where a
  witness connects from, not that its captures are independent.
- Reputation is informational only; it doesn't affect assignment or
  verdicts.

## [1.0.0] - 2026-09-28

The first stable release: a working network of independent witnesses that
record web pages, seal what they saw, catch silent edits and cloaking, and
produce evidence anyone can verify offline.

### Capturing and evidence

- Capture pages over HTTP, or in headless Chromium with `--features render`,
  behind a guard that refuses private and loopback addresses.
- A normalizer that strips ads, cookie banners, trackers, clocks and "posted
  5 minutes ago" noise, with per-site rules and a regression corpus.
- Signed attestations in an append-only, Certificate-Transparency-style
  Merkle log, with inclusion and consistency proofs and a full self-audit.
- Change detection with diffs, and silent-edit detection: a change the page
  doesn't disclose raises an alert.
- Self-contained evidence bundles that `witness verify --bundle` checks
  offline: signature, log inclusion, cosignatures, time bounds, content.
- Time bounds: a drand beacon proves a capture happened after a moment,
  Bitcoin anchoring through OpenTimestamps proves it existed by a later one.
- Retention levels (`full`, `normalized`, `none`), erasure with
  `witness purge` that keeps every proof valid, and WARC export.
- A TLSNotary proof tier (`crates/witness-tlsn`): a second witness
  notarizes the TLS session, so a capture can't be fabricated alone.

### The network

- Federation over HTTPS. Gossip with a random sample of peers; every
  message is signed, and only witnesses in the peer table can send more
  than an announcement.
- Log audits: each log has 16 auditors that check its hourly checkpoints
  with consistency proofs and cosign them. A log that shows two histories
  produces cryptographic proof against itself and is excluded.
- Location corroboration: peers confirm each other's network (ASN) from
  the addresses their connections come from.
- Watch requests assigned by public randomness, at most one witness per
  network, capped per country; requests can be replaced and withdrawn.
- Verdicts that count independent networks: agreed, split (a site showing
  different people different pages) or insufficient, with split alerts.
- Default seed witnesses for joining, with an opt-out for separate
  networks; hourly pruning keeps each node's storage roughly independent
  of the network's size.

### Running it

- A production installer for Debian, Ubuntu, Fedora, RHEL-likes, Arch,
  openSUSE and Alpine, with a hardened systemd or OpenRC service, Caddy
  for automatic TLS, upgrades in place and clean uninstall.
- A container image and compose file.
- `witness net status` names anything that keeps a node out of the
  network. Run as root, `witness` switches to the service user by itself.
- A private web UI with history, diffs, verdicts, alerts and the network.
- Licensed under the GNU AGPL v3.

[2.4.0]: https://git.keepword.net/keepword/keepword/releases/tag/v2.4.0
[2.3.0]: https://git.keepword.net/keepword/keepword/releases/tag/v2.3.0
[2.2.0]: https://git.keepword.net/keepword/keepword/releases/tag/v2.2.0
[2.1.0]: https://git.keepword.net/keepword/keepword/releases/tag/v2.1.0
[2.0.0]: https://git.keepword.net/keepword/keepword/releases/tag/v2.0.0
[1.2.0]: https://git.keepword.net/keepword/keepword/releases/tag/v1.2.0
[1.1.1]: https://git.keepword.net/keepword/keepword/releases/tag/v1.1.1
[1.1.0]: https://git.keepword.net/keepword/keepword/releases/tag/v1.1.0
[1.0.1]: https://git.keepword.net/keepword/keepword/commit/ab4697ece6212704b16d518cc8bd8e7c1e0b3470
[1.0.0]: https://git.keepword.net/keepword/keepword/releases/tag/v1.0.0
