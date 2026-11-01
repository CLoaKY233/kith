#!/bin/sh
# Runs install.sh against a fake release served from disk: it installs both binaries,
# and refuses a tampered archive, one SHA256SUMS does not list, and a bad version.
# Never starts kith.
set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
version=9.9.9
release="$work/releases/v$version"
mkdir -p "$release"

sum() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl \
    aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf; do
    for bin in kith kith-ui; do
        name="$bin-$version-$target"
        mkdir -p "$work/stage/$name"
        printf '#!/bin/sh\necho %s\n' "$bin" > "$work/stage/$name/$bin"
        chmod +x "$work/stage/$name/$bin"
        tar -C "$work/stage" -czf "$release/$name.tar.gz" "$name"
    done
done
(cd "$release" && sum -- *.tar.gz > SHA256SUMS)

run() {
    KITH_VERSION="${2:-$version}" KITH_DOWNLOADS="file://$work/releases" KITH_NO_START=1 \
        KITH_BIN_DIR="$1" KITH_UI=1 sh "$root/install.sh" < /dev/null > "$work/log" 2>&1
}

check() {
    if ! eval "$2"; then
        echo "FAIL: $1"
        cat "$work/log"
        exit 1
    fi
    echo "ok: $1"
}

check "installs kith and kith-ui" \
    'run "$work/bin" && [ "$("$work/bin/kith")" = kith ] && [ "$("$work/bin/kith-ui")" = kith-ui ]'

check "refuses a version that is not one" \
    '! run "$work/bad" "../x" && grep -q "not a release version" "$work/log"'

cp "$release/SHA256SUMS" "$work/SHA256SUMS.orig"
grep -v " kith-$version-" "$work/SHA256SUMS.orig" > "$release/SHA256SUMS"
check "refuses an archive SHA256SUMS does not list" \
    '! run "$work/unlisted" && grep -q "no entry" "$work/log" && [ ! -e "$work/unlisted/kith" ]'

cp "$work/SHA256SUMS.orig" "$release/SHA256SUMS"
for archive in "$release"/kith-"$version"-*.tar.gz; do
    printf 'x' >> "$archive"
done
check "refuses a tampered archive" \
    '! run "$work/tampered" && grep -q "checksum mismatch" "$work/log" && [ ! -e "$work/tampered/kith" ]'
