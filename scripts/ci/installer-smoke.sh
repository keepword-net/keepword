#!/bin/sh
# Installer smoke test for a fresh distro container (no running init):
# install, check the generated service files, run the node the way the
# service would, query its API, then re-run the installer as an upgrade.
# The node never dials the default seeds: it would join the real network.
# Usage: installer-smoke.sh INIT [MODE]   (systemd | openrc | none; source | binary | deb)
# MODE binary installs the latest release, piping the installer to sh the
# way curl | sh does. MODE deb builds from source, then upgrades onto a
# Debian package of that build, the way a re-run moves a node installed
# before the package onto it, and checks that the package's services are
# the ones the installer writes.
set -eu
init=$1
mode=${2:-source}
src=$(cd "$(dirname "$0")/../.." && pwd)

installer() { # installer MODE OPTIONS
    how=$1
    shift
    case "$how" in
        source) sh "$src/scripts/install.sh" --source "$src" "$@" ;;
        binary) sh -s -- --binary "$@" <"$src/scripts/install.sh" ;;
        deb) sh "$src/scripts/install.sh" --deb "$deb" "$@" ;;
    esac
}

packaged() {
    command -v dpkg-query >/dev/null 2>&1 || return 1
    # shellcheck disable=SC2016
    [ "$(dpkg-query -W -f='${Status}' keepword 2>/dev/null)" = "install ok installed" ]
}

first=$mode
[ "$mode" != deb ] || first=source
installer "$first" --yes --init "$init" \
    --no-detect --no-asn-db --asn 64500 --country DE --peer https://peer.example.invalid --no-seeds

keepword --version
test "$(keepword config get network.seeds)" = false
units=/etc/systemd/system
if packaged; then units=/usr/lib/systemd/system; fi
case "$init" in
    systemd) systemd-analyze verify "$units/keepword.service" \
        "$units/keepword-asn-update.service" "$units/keepword-asn-update.timer" ;;
    openrc) test -x /etc/init.d/keepword && sh -n /etc/init.d/keepword ;;
esac

as_keepword() {
    if command -v runuser >/dev/null 2>&1; then runuser -u keepword -- "$@"; else su-exec keepword "$@"; fi
}

as_keepword env KEEPWORD_DIR=/var/lib/keepword HOME=/var/lib/keepword \
    keepword serve --addr 127.0.0.1:8480 --api-addr 127.0.0.1:8481 --watch >/tmp/keepword.log 2>&1 &
pid=$!
i=0
until curl -fsS http://127.0.0.1:8481/v1/descriptor >/tmp/descriptor.json 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 30 ]; then cat /tmp/keepword.log; exit 1; fi
    sleep 1
done
grep -q '"asn":64500' /tmp/descriptor.json
kill "$pid"
wait "$pid" 2>/dev/null || true

# Plain `keepword` as root: it finds the node and becomes the keepword user.
keepword watch add https://example.org/ --every 6h >/dev/null
test "$(stat -c %U /var/lib/keepword/index.sqlite)" = keepword

if [ "$mode" = deb ]; then
    # A package of the build just installed, and what the installer wrote,
    # to hold the package's files against.
    deb=$(sh "$src/packaging/debian/build-deb.sh" "$(command -v keepword)" \
        "$(keepword --version | awk '{print $2}')" "$(dpkg --print-architecture)" /tmp/deb)
    mkdir -p /tmp/installer
    cp /etc/systemd/system/keepword.service /etc/systemd/system/keepword-asn-update.service \
        /etc/systemd/system/keepword-asn-update.timer /usr/local/lib/keepword/update-asn-db /tmp/installer/
fi

key() { keepword id | awk '/witness key/ {print $3}'; }
before=$(key)
installer "$mode" --yes --init "$init" --no-detect --no-asn-db --retain normalized
hash -r
test "$before" = "$(key)"
test "$(keepword config get content.retain)" = normalized
test "$(keepword config get network.seeds)" = false

if [ "$mode" = deb ]; then
    packaged
    test "$(command -v keepword)" = /usr/bin/keepword
    test ! -e /usr/local/bin/keepword
    test ! -e /usr/local/lib/keepword
    test ! -e /etc/systemd/system/keepword.service
    # The default address and data directory need no drop-in.
    test ! -e /etc/systemd/system/keepword.service.d
    test -L /etc/systemd/system/multi-user.target.wants/keepword.service
    systemd-analyze verify /usr/lib/systemd/system/keepword.service \
        /usr/lib/systemd/system/keepword-asn-update.service /usr/lib/systemd/system/keepword-asn-update.timer
    for f in keepword.service keepword-asn-update.service keepword-asn-update.timer update-asn-db; do
        case "$f" in
            update-asn-db) pkg=/usr/lib/keepword/$f ;;
            *) pkg=/usr/lib/systemd/system/$f ;;
        esac
        grep -v '^#' "/tmp/installer/$f" \
            | sed 's|/usr/local/bin/|/usr/bin/|; s|/usr/local/lib/keepword/|/usr/lib/keepword/|' >/tmp/want
        grep -v '^#' "$pkg" >/tmp/got
        diff -u /tmp/want /tmp/got || { echo "$pkg differs from what the installer writes" >&2; exit 1; }
    done
fi

sh "$src/scripts/install.sh" --uninstall --purge --yes --init "$init"
if id keepword >/dev/null 2>&1; then echo "the keepword user still exists" >&2; exit 1; fi
if packaged; then echo "the keepword package is still installed" >&2; exit 1; fi
test ! -e /etc/apt/sources.list.d/keepword.sources
echo "installer smoke test passed ($init, $mode)"
