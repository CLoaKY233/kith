#!/bin/sh
# Installs kith on macOS or Linux and turns it on.
#
#   curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | sh
#
# KITH_VERSION=0.1.0   install that release instead of the latest
# KITH_NO_START=1      install only; run `kith up` yourself later
# KITH_DOWNLOADS=url   fetch releases from a mirror instead of GitHub
# KITH_BIN_DIR=dir     install somewhere other than /usr/local/bin
set -eu

repo="CLoaKY233/kith"
downloads="${KITH_DOWNLOADS:-https://github.com/$repo/releases/download}"
bin_dir="${KITH_BIN_DIR:-/usr/local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf 'kith install: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || fail "needs $1"
}

as_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        need sudo
        sudo "$@"
    fi
}

target() {
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch" in
        Darwin/arm64) echo aarch64-apple-darwin ;;
        Darwin/x86_64) echo x86_64-apple-darwin ;;
        Linux/x86_64 | Linux/amd64) echo x86_64-unknown-linux-musl ;;
        Linux/aarch64 | Linux/arm64) echo aarch64-unknown-linux-musl ;;
        Linux/armv7l | Linux/armv7) echo armv7-unknown-linux-musleabihf ;;
        *) fail "no release for $os on $arch; build from source: cargo build --release -p kith-cli" ;;
    esac
}

latest_version() {
    url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")" ||
        fail "cannot reach GitHub to find the latest release"
    tag="${url##*/}"
    case "$tag" in
        v*) echo "${tag#v}" ;;
        *) fail "no release published yet" ;;
    esac
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

need curl
need tar
need uname
target="$(target)"
version="${KITH_VERSION:-$(latest_version)}"
version="${version#v}"
name="kith-$version-$target"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "downloading kith $version for $target"
curl -fsSL -o "$tmp/$name.tar.gz" "$downloads/v$version/$name.tar.gz" ||
    fail "download failed: $downloads/v$version/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS" "$downloads/v$version/SHA256SUMS" ||
    fail "could not download the checksums"

expected="$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $name.tar.gz"
[ "$(sha256 "$tmp/$name.tar.gz")" = "$expected" ] ||
    fail "checksum mismatch for $name.tar.gz; not installing"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
as_root mkdir -p "$bin_dir"
as_root install -m 0755 "$tmp/$name/kith" "$bin_dir/kith"
say "installed $bin_dir/kith"

if [ "$(uname -s)" = Linux ] && command -v groupadd >/dev/null 2>&1; then
    if ! getent group kith >/dev/null 2>&1; then
        as_root groupadd --system kith
    fi
    user="${SUDO_USER:-${USER:-}}"
    if [ -n "$user" ] && [ "$user" != root ] && ! id -nG "$user" | tr ' ' '\n' | grep -qx kith; then
        as_root usermod -aG kith "$user"
        say "added $user to the kith group; log out and back in to use kith without sudo"
    fi
fi

if pgrep -x tailscaled >/dev/null 2>&1 || pgrep -f 'Tailscale' >/dev/null 2>&1; then
    say "note: Tailscale is running; it uses the same 100.64.0.0/10 addresses, so quit it while kith is on"
fi

if [ -n "${KITH_NO_START:-}" ]; then
    say "turn it on with: kith up"
    exit 0
fi

as_root "$bin_dir/kith" up
say ""
say "next:"
say "  kith create home        start a network here"
say "  kith join <link>        or join one with an invite link"
say "  kith --help             everything else"
