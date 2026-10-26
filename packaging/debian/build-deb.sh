#!/bin/sh
# Builds the keepword Debian package around a binary, a release's static
# build in .forgejo/workflows/release.yml: the command, the services the
# installer writes, and the IP-to-ASN refresh. The package makes no node;
# the installer does, and on Debian and Ubuntu installs this package.
# Usage: build-deb.sh BINARY VERSION ARCH OUTDIR
#   VERSION is the crate's (2.5.0-beta.4), ARCH Debian's (amd64, arm64).
#   Prints the path of the .deb it wrote.
set -eu
umask 022
[ $# -eq 4 ] || { echo "usage: build-deb.sh BINARY VERSION ARCH OUTDIR" >&2; exit 2; }
bin=$1
version=$2
arch=$3
out=$4
here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$here/../.." && pwd)

# A pre-release sorts before its release: 2.5.0-beta.4 is 2.5.0~beta.4.
debver=$(printf '%s' "$version" | sed 's/-/~/')
case "$debver" in ''|*-*|*[!0-9A-Za-z.~+]*) echo "no Debian version for $version" >&2; exit 2 ;; esac

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
chmod 0755 "$root"
install -D -m 0755 "$bin" "$root/usr/bin/keepword"
install -D -m 0755 "$here/update-asn-db" "$root/usr/lib/keepword/update-asn-db"
for unit in keepword.service keepword-asn-update.service keepword-asn-update.timer; do
    install -D -m 0644 "$here/$unit" "$root/usr/lib/systemd/system/$unit"
done

doc=$root/usr/share/doc/keepword
install -d "$doc"
{
    cat <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Keepword
Source: https://git.keepword.net/keepword/keepword

Files: *
Copyright: The Keepword contributors
License: AGPL-3.0-only
EOF
    sed 's/^/ /; s/^ *$/ ./' "$src/LICENSE"
} >"$doc/copyright"
gzip -9n <"$src/CHANGELOG.md" >"$doc/changelog.gz"
chmod 0644 "$doc/copyright" "$doc/changelog.gz"

install -d "$root/DEBIAN"
for script in postinst prerm postrm; do
    install -m 0755 "$here/$script" "$root/DEBIAN/$script"
done
size=$(du -sk --exclude=DEBIAN "$root" | cut -f1)
cat >"$root/DEBIAN/control" <<EOF
Package: keepword
Version: $debver
Architecture: $arch
Maintainer: Keepword <security@keepword.net>
Installed-Size: $size
Depends: adduser, ca-certificates, curl
Section: net
Priority: optional
Homepage: https://keepword.net
Description: witness that keeps proof of what web pages said
 Keepword is a network of independent witnesses that record web pages,
 seal what they saw and catch silent edits. Anyone can check the proof,
 offline, without trusting anyone.
 .
 This package installs the keepword command and the node's services, which
 stay off until a node is set up. The installer sets one up, and installs
 this package on Debian and Ubuntu: https://keepword.net/install/
EOF

mkdir -p "$out"
deb="$out/keepword_${debver}_$arch.deb"
dpkg-deb --root-owner-group --build "$root" "$deb" >/dev/null
printf '%s\n' "$deb"
