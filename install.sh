#!/bin/sh
# Installs kith on macOS or Linux and turns it on.
#
#   curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | sh
#
# KITH_VERSION=0.1.0   install that release instead of the latest
# KITH_NO_START=1      install only; run `kith up` yourself later
# KITH_DOWNLOADS=url   fetch releases from a mirror instead of GitHub
# KITH_BIN_DIR=dir     install somewhere other than /usr/local/bin
# KITH_UI=1            also install the web UI (kith ui) without asking
#
# On a Linux server, also run the kith Console, reachable at a domain over HTTPS:
#
#   curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | sh -s -- --console vpn.example.com
#
# KITH_CONSOLE=domain        the same as --console domain
# KITH_CONSOLE_EMAIL=address contact address for Let's Encrypt (optional)
# KITH_NETWORK=name          the network to create when this device is in none
#                            (default: the domain's first label)
set -eu

console="${KITH_CONSOLE:-}"
while [ $# -gt 0 ]; do
    case "$1" in
        --console)
            [ $# -ge 2 ] || { printf 'kith install: --console needs a domain\n' >&2; exit 1; }
            console="$2"
            shift 2
            ;;
        --console=*)
            console="${1#--console=}"
            shift
            ;;
        *)
            printf 'kith install: unknown option %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

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

# Fetches $1 into $2. Over https, redirects may not fall back to plain http.
fetch() {
    case "$1" in
        https://*) curl -fsSL --proto-redir '=https' -o "$2" "$1" ;;
        *) curl -fsSL -o "$2" "$1" ;;
    esac
}

need curl
need tar
need uname
need install
if [ -n "$console" ]; then
    [ "$(uname -s)" = Linux ] ||
        fail "--console sets up a Linux server; on macOS, use kith ui for this computer"
    case "$console" in
        *[!A-Za-z0-9.-]* | [.-]* | *. | *..*) fail "not a domain: $console" ;;
        *.*) ;;
        *) fail "not a domain: $console" ;;
    esac
    command -v systemctl >/dev/null 2>&1 || fail "--console needs systemd"
    [ -z "${KITH_NO_START:-}" ] || fail "--console starts kith; unset KITH_NO_START"
fi
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 ||
    fail "needs sha256sum or shasum to verify the download"
target="$(target)"
version="${KITH_VERSION:-$(latest_version)}"
version="${version#v}"
case "$version" in
    "" | *[!0-9A-Za-z.+-]*) fail "not a release version: $version" ;;
esac
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fetch "$downloads/v$version/SHA256SUMS" "$tmp/SHA256SUMS" ||
    fail "could not download the checksums"

# Downloads, verifies and installs the release binary $1 into $bin_dir.
install_bin() {
    name="$1-$version-$target"
    say "downloading $1 $version for $target"
    fetch "$downloads/v$version/$name.tar.gz" "$tmp/$name.tar.gz" ||
        fail "download failed: $downloads/v$version/$name.tar.gz"
    expected="$(awk -v file="$name.tar.gz" '$2 == file || $2 == "*" file { print $1; exit }' "$tmp/SHA256SUMS")"
    [ -n "$expected" ] || fail "SHA256SUMS has no entry for $name.tar.gz"
    [ "$(sha256 "$tmp/$name.tar.gz")" = "$expected" ] ||
        fail "checksum mismatch for $name.tar.gz; not installing"
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp" "$name/$1" ||
        fail "$name.tar.gz has no $name/$1"
    if mkdir -p "$bin_dir" 2>/dev/null && [ -w "$bin_dir" ]; then
        install -m 0755 "$tmp/$name/$1" "$bin_dir/$1"
    else
        as_root mkdir -p "$bin_dir"
        as_root install -m 0755 "$tmp/$name/$1" "$bin_dir/$1"
    fi
    say "installed $bin_dir/$1"
}

install_bin kith

ui="${KITH_UI:-}"
[ -z "$console" ] || ui=1
case "$target" in
    armv7-*)
        [ -z "$console" ] || fail "no kith-ui build for $target, so no Console"
        [ "$ui" = 1 ] && say "no web UI build for $target; skipping kith-ui"
        ui=0
        ;;
esac
if [ -z "$ui" ]; then
    if [ -t 0 ]; then
        printf 'Also install the web UI (kith ui)? [y/N] '
        read -r answer || answer=""
        case "$answer" in
            y | Y | yes | Yes) ui=1 ;;
        esac
    else
        say "to also install the web UI (kith ui), run the installer with KITH_UI=1:"
        say "  curl -fsSL https://raw.githubusercontent.com/$repo/main/install.sh | KITH_UI=1 sh"
    fi
fi
if [ "$ui" = 1 ]; then
    install_bin kith-ui
fi

if [ "$(uname -s)" = Linux ] && command -v groupadd >/dev/null 2>&1; then
    if ! grep -q '^kith:' /etc/group 2>/dev/null && ! getent group kith >/dev/null 2>&1; then
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

# Runs kith-ui as the Console at $console: a network if there is none, the service, and
# the one-time setup link it writes.
setup_console() {
    tries=0
    until as_root "$bin_dir/kith" status --json >"$tmp/status.json" 2>/dev/null; do
        tries=$((tries + 1))
        [ "$tries" -lt 20 ] || fail "kith did not start; see: kith doctor"
        sleep 1
    done
    if grep -q '"networks":\[\]' "$tmp/status.json"; then
        network="${KITH_NETWORK:-${console%%.*}}"
        as_root "$bin_dir/kith" create "$network"
        say "created the network $network"
    fi

    unit="$tmp/kith-console.service"
    here="$(dirname "$0")/packaging/systemd/kith-console.service"
    if [ -f "$here" ]; then
        cp "$here" "$unit"
    else
        fetch "https://raw.githubusercontent.com/$repo/v$version/packaging/systemd/kith-console.service" "$unit" ||
            fetch "https://raw.githubusercontent.com/$repo/main/packaging/systemd/kith-console.service" "$unit" ||
            fail "could not download the Console's service file"
    fi
    sed "s|/usr/local/bin/kith-ui|$bin_dir/kith-ui|" "$unit" >"$unit.local"

    {
        printf 'KITH_CONSOLE_DOMAIN=%s\n' "$console"
        printf 'KITH_CONSOLE_EMAIL=%s\n' "${KITH_CONSOLE_EMAIL:-}"
    } >"$tmp/console.env"
    # A real system user, not systemd's DynamicUser: kith checks the operator group through
    # /etc/passwd and /etc/group, where dynamic users never appear.
    if ! id kith-console >/dev/null 2>&1; then
        as_root useradd --system --no-create-home --home-dir /var/lib/kith-console \
            --shell /usr/sbin/nologin -G kith kith-console
    fi
    as_root mkdir -p /etc/kith
    as_root install -m 0644 "$tmp/console.env" /etc/kith/console.env
    as_root install -m 0644 "$unit.local" /etc/systemd/system/kith-console.service
    link_file=/var/lib/kith-console/setup-link
    as_root rm -f "$link_file"
    as_root systemctl daemon-reload
    as_root systemctl enable kith-console.service >/dev/null 2>&1
    as_root systemctl restart kith-console.service

    tries=0
    while ! as_root test -s "$link_file"; do
        tries=$((tries + 1))
        if [ "$tries" -ge 15 ]; then
            if as_root systemctl is-active --quiet kith-console.service; then
                say "the Console is already set up: https://$console"
                return
            fi
            fail "the Console did not start; see: journalctl -u kith-console"
        fi
        sleep 1
    done
    say ""
    say "Open this link within 30 minutes to set up the kith Console. It works once:"
    say "  $(as_root cat "$link_file")"
    say "Ports 80 and 443 must be open, and $console must point at this server."
    say "A new link: sudo systemctl restart kith-console, then sudo cat $link_file"
}

if [ -n "$console" ]; then
    setup_console
    exit 0
fi

say ""
say "next:"
say "  kith create home        start a network here"
say "  kith join <link>        or join one with an invite link"
say "  kith --help             everything else"
