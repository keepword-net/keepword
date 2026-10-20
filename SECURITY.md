# Security

Keepword's job is to be believed. A bug that lets someone fake a record, or
take a witness down from the outside, matters more here than in most
software, so thank you for looking.

## Reporting a vulnerability

Please **don't** open a public issue. Email **security@keepword.net**
with:

- what you found and what an attacker gets from it,
- how to reproduce it: the commands, the page or the bundle, or a test,
- the Keepword version (`keepword --version`) or the commit you tested.

You'll get a reply. The fix is coordinated with you before anything is made
public, and you're credited in the changelog unless you'd rather not be.

### Test on your own nodes

Run the witnesses you attack yourself, with the public network turned off
(`keepword config set network.seeds false`, see
[CONTRIBUTING.md](CONTRIBUTING.md#dont-join-the-real-network-by-accident)).
Don't test against witnesses other people run, including the project's
seeds: they are the public network, and what you do to them reaches
everyone's records.

## What counts

Anything that breaks a promise in [the threat model](docs/DESIGN.md#1-threat-model).
In particular, anything that lets someone:

- **forge or backdate evidence**: a statement a witness didn't sign, or a
  time it couldn't have been made at,
- **make `keepword verify` accept a bundle it shouldn't**, or report a
  check as passed that didn't,
- **rewrite or fork a log without getting caught** by the witnesses that
  audit it,
- **fake a verdict**: make one key count as several networks, steer which
  witnesses are assigned a request or drawn for a recheck, or get an
  honest witness charged with a failed claim,
- **make a relay do more than withhold**: read, change or answer for the
  witnesses it relays for,
- **make a witness fetch addresses it shouldn't**, such as its own network,
  localhost or cloud metadata,
- **crash a witness or exhaust its memory, disk or connections** from a
  page it captures or from a peer,
- **run code or read files** through the web UI, the peer API, the
  installer or the container image.

When in doubt, email.

## What doesn't

These are known limits, written down in the design. They're worth an
issue, or a pull request, but not an embargo:

- **One operator who really connects from many networks** counts as many
  witnesses ([DESIGN.md §6.3](docs/DESIGN.md#63-vantage-corroboration)).
- **Evidence from a single witness** proves only what that witness signed.
  `keepword verify` says so.
- **Normalizer mistakes**: a silent edit raised when nothing changed, or
  one missed. Report them as
  [issues](https://git.keepword.net/keepword/keepword/issues) with the
  URL and `keepword diff`.
- **A site that blocks witnesses**, or shows them A/B variants. Blocking is
  visible, and variants are an [open question](docs/DESIGN.md#9-open-questions).
- **Bugs in what Keepword builds on**, such as drand, OpenTimestamps or
  TLSNotary, belong with those projects. Do tell us if Keepword uses one
  in a way that makes it worse.

## Supported versions

Fixes go into a new release of the latest version, and the changelog
says what was fixed and who should upgrade. Older versions don't get
backports, so keep witnesses on the latest release: re-running the
installer, or pulling `git.keepword.net/keepword/keepword:2`, upgrades
one.

A fix that changes the protocol can take longer to release, because
witnesses on the old and new versions have to keep federating while
operators upgrade.
