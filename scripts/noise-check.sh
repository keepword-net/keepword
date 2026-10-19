#!/bin/sh
# Capture live pages twice and show those that normalize differently: noise
# the normalizer misses, which would raise a silent-edit alert on every
# capture. Reads URLs one per line, or `keepword watch list` output.
# Usage: noise-check.sh [FILE]   (else stdin; exits 1 if a page changed)
#   KEEPWORD  binary to run (default: keepword)
#   GAP       seconds between the two rounds (default: 90)
set -euf
kw=${KEEPWORD:-keepword}
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
"$kw" --dir "$dir" init >/dev/null
urls=$(awk '/^https?:\/\// { print $1 }' "${1:--}")

for round in 1 2; do
    [ "$round" = 1 ] || sleep "${GAP:-90}"
    for u in $urls; do
        "$kw" --dir "$dir" capture "$u" >/dev/null || echo "capture failed: $u" >&2
    done
done

changed=0
for u in $urls; do
    if out=$("$kw" --dir "$dir" diff "$u" 2>/dev/null); then
        printf '=== %s\n%s\n\n' "$u" "$out"
        changed=1
    fi
done
exit "$changed"
