# Installing Keepword

There are two supported ways to run a node in production:

- **The installer** (`scripts/install.sh`) downloads the latest release's
  static binary, or builds Keepword from source, and sets it up as a hardened
  system service. It supports Debian, Ubuntu, Fedora, RHEL-likes, Arch,
  openSUSE and Alpine, with systemd or OpenRC.
- **The container image** (`git.keepword.net/keepword/keepword`) is for
  Docker, Podman, NAS boxes and Kubernetes.

Stuck, or not sure which options fit? Ask in
[#operators:keepword.net](https://matrix.to/#/#operators:keepword.net) on
Matrix.

## Installer

```sh
curl -fsSL https://git.keepword.net/keepword/keepword/raw/branch/main/scripts/install.sh \
    | sudo sh -s -- --domain witness.example.org --caddy
```

This installs the latest release's static binary (x86_64 or arm64), with no
checkout, compiler or build; `--ref vX.Y.Z` picks another release. `--build`
builds from source instead: the checkout the installer runs from, or piped,
a clone of `main`, which can be ahead of the latest release (add
`--ref vX.Y.Z` to build a release). Release binaries don't include headless
rendering or the TLSNotary tier, so `--render` and `--notary` always build:

```sh
git clone https://git.keepword.net/keepword/keepword.git
cd keepword
sudo sh scripts/install.sh --build --domain witness.example.org --caddy
```

On Debian and Ubuntu with systemd, the installer installs the release as a
Debian package from Keepword's apt repository, so `apt upgrade` keeps the
node current with the rest of the system: see
[The Debian package](#the-debian-package).

The installer:

1. installs dependencies with the system package manager: just `curl` and
   `tar`, or build tools with `--build` (and Caddy when you pass `--caddy`).
   On Arch this is a full `pacman -Syu`, because Arch doesn't support
   partial upgrades; use `--no-packages` to manage packages yourself;
2. installs Rust into `/opt/keepword` if the system Rust is missing or too
   old, verifying the `rustup-init` checksum (only with `--build`);
3. downloads the release binary and checks it against the release's
   `SHA256SUMS`, or with `--build` builds `keepword` in release mode, and
   installs it to `/usr/local/bin`; or installs the Debian package;
4. creates a `keepword` system user and `/var/lib/keepword` (mode 0750);
5. creates the node: key, config and empty log;
6. downloads the IP-to-ASN table from iptoasn.com and schedules a weekly
   refresh;
7. detects the node's ASN and country from its public IP, unless you pass
   `--asn`/`--country` or `--no-detect`;
8. configures the endpoint, peers and reverse-proxy trust;
9. installs and starts a sandboxed service, then checks that the API
   answers.

Re-running the installer **upgrades in place**: it downloads the latest
release (or rebuilds with `--build`), reinstalls and restarts, and never touches the key or data. Options given on the re-run are
applied to the existing configuration.

### Common setups

```sh
# A public witness behind Caddy with automatic TLS, joining two peers
sudo sh scripts/install.sh --domain witness.example.org --caddy \
    --peer https://witness.one.example --peer https://witness.two.example

# A public witness behind your own reverse proxy
sudo sh scripts/install.sh --domain witness.example.org

# A public witness without a domain: the API directly on port 8481,
# advertised as http://<this server's IP>:8481
sudo sh scripts/install.sh --public-api 0.0.0.0:8481

# A witness at home, behind a router: peers reach it through one of the
# witnesses it knows, which relay for others
sudo sh scripts/install.sh --relay auto

# A private watchdog: no network, hashes and diffs only
sudo sh scripts/install.sh --no-asn-db

# Everything: headless rendering and a TLSNotary notary on :8482
sudo sh scripts/install.sh --domain witness.example.org --caddy --render --notary
```

`sh scripts/install.sh --help` lists every option. `--source DIR` builds a
local checkout. `--build` builds the checkout the installer runs from, or
clones `--repo`/`--ref` into `/opt/keepword/src`.

### What gets installed

| Path | What |
|---|---|
| `/usr/local/bin/keepword`, `keepword-tlsn` | binaries |
| `/usr/local/lib/keepword/update-asn-db` | IP-to-ASN refresh script |
| `/var/lib/keepword/` | key, `keepword.toml`, SQLite index, blobs, ASN table |
| `/opt/keepword/` | with `--build`: Rust toolchain, build cache, cloned source |
| `keepword.service` | the node: `serve --watch --anchor`, UI on 127.0.0.1:8480, API on 127.0.0.1:8481 (or the `--public-api` address) |
| `keepword-asn-update.timer` | weekly ASN table refresh (cron on OpenRC) |
| `keepword-notary.service` | with `--notary` |
| `/etc/caddy/keepword.caddy` | with `--caddy`; imported from the Caddyfile |

The systemd units run as the unprivileged `keepword` user. Hardening:
read-only system, no home directories, no capabilities, a system-call
allow-list, no new privileges, and memory that can't be both writable and
executable (except with `--render`, because Chromium's JIT needs it).

### The Debian package

Releases from 2.5.0 on are also Debian packages, for amd64 and arm64, in
Keepword's apt repository at
`https://git.keepword.net/api/packages/keepword/debian`. Its `stable` suite
has the releases, and `beta` has them and the pre-releases. On Debian and
Ubuntu with systemd, the installer installs the package from there instead of
the release archive, and adds the repository: `stable`, or `beta` when
`--ref` is a pre-release, so that node gets the pre-releases after it and
then the release. `apt upgrade` keeps the node current, and re-running the
installer still upgrades it too. A node installed before, built or from the
archive, moves onto the package when you re-run the installer without
`--build`, keeping its key and data. `--no-apt` installs the release archive
instead and moves a node off the package; `--deb FILE` installs a package
you have, such as one from a release's downloads, without the repository.

The package holds `/usr/bin/keepword`, the IP-to-ASN refresh in
`/usr/lib/keepword/`, and in `/usr/lib/systemd/system/` the same services the
installer writes for the other ways. It sets up no node and starts nothing by
itself: installed on its own, it gives you the `keepword` command, enough to
verify evidence, and the installer sets up the node. Where a node differs from
the package's services, with `--public-api` or `--data-dir`, the installer
adds a drop-in, `/etc/systemd/system/keepword.service.d/installer.conf`. An
upgrade restarts a running node, `apt remove keepword` stops it, and even
`apt purge` keeps `/var/lib/keepword`.

To add the repository yourself:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
sudo curl -fsSL https://git.keepword.net/api/packages/keepword/debian/repository.key \
    -o /etc/apt/keyrings/keepword.asc
sudo tee /etc/apt/sources.list.d/keepword.sources >/dev/null <<'EOF'
Types: deb
URIs: https://git.keepword.net/api/packages/keepword/debian
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/keepword.asc
EOF
sudo apt update && sudo apt install keepword
```

### Day-to-day

Run `keepword` as root or with sudo. It finds the installed node by itself
and switches to the `keepword` user before touching anything, so file
ownership stays right. No alias or `KEEPWORD_DIR` is needed. As an ordinary
user it tells you to use sudo, since the data directory is private to the
service.

```sh
keepword watch add https://example.org/terms --every 6h
keepword request https://example.org/terms --every 1h --for 7days   # the network watches it
keepword request https://example.org/terms --cancel
keepword net status                     # ends with warnings about anything misconfigured
keepword config show
keepword config set network.peers '["https://witness.one.example"]'
sudo systemctl restart keepword         # after changing the config
journalctl -u keepword -f
```

The web UI shows captured content and runs the node, so it only listens on
localhost. View it with an SSH tunnel: `ssh -L 8480:127.0.0.1:8480
your-server`, then open http://localhost:8480. Settings you change there
apply when it offers to restart the service. To reach it from your network
instead, see "The web UI" in [USAGE.md](USAGE.md): it then asks for a
password.

To start a network or join one, see [NETWORK.md](NETWORK.md).

**Back up `/var/lib/keepword/witness.key`.** It is the witness's identity, and
the log is only verifiable against it. A new key is a new witness.

### Firewall

| Port | Open to | Why |
|---|---|---|
| 443 (and 80 for ACME) | everyone | peer API through the reverse proxy |
| 8481 | everyone, only with `--public-api` | peer API without a proxy |
| 8482 | peers, only with `--notary` | TLSNotary sessions (raw TCP) |
| 8480 | nobody | web UI, localhost only |

### Your own reverse proxy

The proxy must forward `/v1/` to `127.0.0.1:8481` and put the real client
address in `X-Forwarded-For`. The installer turns on
`network.trust_forwarded_for` when you pass `--domain`, because observation
receipts, and with them every peer's corroborated location, depend on it.
The node uses the **last** address in that header, the one your proxy
added. It only reads the header on connections from this machine or a
private network, so the proxy must run there: with the API exposed
directly, clients could otherwise claim any address.

nginx:

```nginx
server {
    listen 443 ssl http2;
    server_name witness.example.org;
    # ssl_certificate ...; ssl_certificate_key ...;

    location /v1/ {
        proxy_pass http://127.0.0.1:8481;
        proxy_set_header X-Forwarded-For $remote_addr;   # replace, don't append
        proxy_set_header Host $host;
    }
    location / { return 404; }
}
```

### Behind Cloudflare or another CDN

Through a CDN, the address your proxy sees, and adds to `X-Forwarded-For`,
is the CDN's, and the node would place every peer in the CDN's network
(`keepword net status` warns when peers seem to connect from Cloudflare).
Have the proxy work out the client's address, as for any site behind a
CDN, and pass it on in a header it sets itself. Caddy:

```caddy
{
    servers {
        trusted_proxies cloudflare     # caddy-cloudflare-ip module; or: static <Cloudflare's ranges>
        client_ip_headers CF-Connecting-IP
    }
}

witness.example.org {
    reverse_proxy 127.0.0.1:8481 {
        header_up X-Real-IP {client_ip}
    }
}
```

With nginx, `set_real_ip_from` Cloudflare's ranges and `real_ip_header
CF-Connecting-IP`, then `proxy_set_header X-Real-IP $remote_addr`. Then
tell the node which header it is:

```sh
keepword config set network.client_ip_header X-Real-IP
sudo systemctl restart keepword
```

The proxy must set that header on every request, replacing any the client
sent (`header_up` and `proxy_set_header` do). Don't name a header the proxy
only passes on, such as `CF-Connecting-IP`: anyone reaching your server
around the CDN could write it. Peers are programs, not browsers: keep the
CDN from challenging requests to `/v1/` (Cloudflare's Bot Fight Mode,
"Under Attack" mode).

### Uninstalling

```sh
sudo sh scripts/install.sh --uninstall           # keeps /var/lib/keepword
sudo sh scripts/install.sh --uninstall --purge   # deletes the key and log too
```

On a node with the Debian package, both remove the package and the apt
repository too.

## Container

Releases from 2.4.0 on publish the image for x86_64 and arm64, with the
release's static binary in it. It is tagged with the version (`2.4.0`),
its minor and major versions (`2.4`, `2`) and `latest`:

```sh
docker run -d --name keepword --restart unless-stopped \
    -v keepword-data:/data \
    -p 127.0.0.1:8480:8480 -p 127.0.0.1:8481:8481 \
    -e KEEPWORD_ENDPOINT=https://witness.example.org \
    -e KEEPWORD_PEERS=https://witness.one.example \
    -e KEEPWORD_BEHIND_PROXY=1 \
    git.keepword.net/keepword/keepword:2
```

Or `docker compose -f packaging/docker/compose.yaml up -d`, which pulls
the same image; to upgrade, `pull` and `up -d` again. To build the image
from a checkout instead, `docker build -t keepword .` and run `keepword`,
or add `--build` to `up`. Settings:

| Variable | Meaning |
|---|---|
| `KEEPWORD_ASN`, `KEEPWORD_COUNTRY` | where the node is (first start) |
| `KEEPWORD_RETAIN` | `full`, `normalized` (default) or `none` (first start) |
| `KEEPWORD_ENDPOINT` | public URL of the peer API |
| `KEEPWORD_RELAY` | instead of an endpoint, at home: `auto`, or a witness that relays for this one |
| `KEEPWORD_PEERS` | comma-separated bootstrap peers |
| `KEEPWORD_SEEDS=0` | don't join through the default seeds (a separate network, or a test) |
| `KEEPWORD_BEHIND_PROXY=1` | trust `X-Forwarded-For` from your proxy |
| `KEEPWORD_CLIENT_IP_HEADER` | the header your proxy puts the client address in, e.g. `X-Real-IP` (behind a CDN) |
| `KEEPWORD_ASN_DB=0` | skip the IP-to-ASN table (refreshed at start when older than a week) |
| `KEEPWORD_UI_PASSWORD` | the web UI's password; else the first visit sets one |
| `KEEPWORD_UI_AUTH=none` | no password, for a proxy in front of the UI that asks for one |

In the container the web UI listens on all addresses, so it asks for a
password. Settings changed in it apply after its restart button, which
restarts the container. Those set by the variables above are shown there
but changed here, since the container applies them on every start.

The `/data` volume holds the key: back it up. Run CLI commands with
`docker exec keepword keepword …`. With Docker's userland proxy, peers can
appear to connect from the gateway address. Put a TLS reverse proxy on the
host in front of port 8481 and set `KEEPWORD_BEHIND_PROXY=1`.
