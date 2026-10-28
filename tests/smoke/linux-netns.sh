#!/bin/sh
# End-to-end smoke test on a real Linux kernel, in network namespaces:
#
#   [inet 203.0.113.1] --uplink-- [srv 203.0.113.2 / br0 10.99.0.1] --br0--+-- [a 10.99.0.2]
#                                  exit + relay, DNS stub for the LAN       +-- [b 10.99.0.3]
#
# srv forwards like a Docker host (policy drop, DOCKER-USER first), so the exit only works
# if kith opens DOCKER-USER. Every node must leave its namespace exactly as it found it.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
KITH=${KITH:-$ROOT/target/release/kith}
WORK=$(mktemp -d)
NET=home
PORT=41641
MESH_DNS=100.64.0.1
EXTERNAL_NAME=example.test
EXTERNAL_ADDR=203.0.113.80
NODES="srv a b"

log() { printf '\n==> %s\n' "$*"; }
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

ns() { printf 'kith-smoke-%s' "$1"; }
# POSIX sh has no locals, so helpers use names no caller loops over.
in_ns() {
    _ns=$1
    shift
    ip netns exec "$(ns "$_ns")" "$@"
}
kith_in() {
    _kn=$1
    shift
    in_ns "$_kn" env KITH_SOCKET="$WORK/$_kn/kith.sock" "$KITH" "$@"
}

dump_logs() {
    for n in $NODES; do
        [ -f "$WORK/$n/daemon.log" ] || continue
        printf '\n----- %s daemon log -----\n' "$n"
        tail -n 80 "$WORK/$n/daemon.log"
        printf '\n----- %s status -----\n' "$n"
        kith_in "$n" status 2>&1 || true
    done
}

cleanup() {
    status=$?
    trap - EXIT INT TERM
    if [ "$status" -ne 0 ]; then
        printf '\nFAILED (exit %s)\n' "$status" >&2
        dump_logs >&2
    fi
    for n in $NODES; do
        [ -f "$WORK/$n/pid" ] && kill "$(cat "$WORK/$n/pid")" 2>/dev/null || true
    done
    sleep 1
    for n in $NODES inet; do
        ip netns del "$(ns "$n")" 2>/dev/null || true
        rm -rf "/etc/netns/$(ns "$n")"
    done
    rm -rf "$WORK"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

retry() {
    what=$1
    tries=$2
    shift 2
    i=0
    until "$@" >/dev/null 2>&1; do
        i=$((i + 1))
        [ "$i" -lt "$tries" ] || fail "timed out: $what"
        sleep 1
    done
}

[ "$(id -u)" -eq 0 ] || fail "run as root"
for tool in ip nft jq ping dig python3 unshare; do
    command -v "$tool" >/dev/null || fail "missing $tool"
done
[ -x "$KITH" ] || fail "build kith first: cargo build -p kith-cli --release"

log "building the namespaces"
for n in $NODES inet; do
    ip netns add "$(ns "$n")"
    in_ns "$n" ip link set lo up
    mkdir -p "/etc/netns/$(ns "$n")" "$WORK/$n/state"
    printf 'nameserver 10.99.0.1\n' >"/etc/netns/$(ns "$n")/resolv.conf"
done
ip link add kith-up type veth peer name kith-up-in
ip link set kith-up netns "$(ns srv)"
ip link set kith-up-in netns "$(ns inet)"
in_ns inet ip addr add 203.0.113.1/24 dev kith-up-in
in_ns inet ip link set kith-up-in up
in_ns srv ip addr add 203.0.113.2/24 dev kith-up
in_ns srv ip link set kith-up up
in_ns srv ip route add default via 203.0.113.1
in_ns srv ip link add br0 type bridge
in_ns srv ip addr add 10.99.0.1/24 dev br0
in_ns srv ip link set br0 up
i=2
for n in a b; do
    ip link add "kith-$n" type veth peer name "kith-$n-br"
    ip link set "kith-$n" netns "$(ns "$n")"
    ip link set "kith-$n-br" netns "$(ns srv)"
    in_ns srv ip link set "kith-$n-br" master br0 up
    in_ns "$n" ip addr add "10.99.0.$i/24" dev "kith-$n"
    in_ns "$n" ip link set "kith-$n" up
    i=$((i + 1))
done
in_ns a ping -c 1 -W 2 10.99.0.3 >/dev/null || fail "LAN is down"

log "srv forwards like a Docker host: policy drop, DOCKER-USER first"
in_ns srv nft -f - <<'NFT'
table ip filter {
    chain DOCKER-USER { }
    chain FORWARD {
        type filter hook forward priority filter; policy drop;
        jump DOCKER-USER
    }
}
NFT

log "a DNS stub on srv answers $EXTERNAL_NAME for names outside the mesh"
cat >"$WORK/dns.py" <<'PY'
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("10.99.0.1", 53))
answer = bytes(int(x) for x in sys.argv[1].split("."))
while True:
    q, peer = s.recvfrom(512)
    end = q.index(b"\0", 12) + 5
    reply = q[:2] + b"\x81\x80" + q[4:6] + b"\0\x01\0\0\0\0" + q[12:end]
    reply += b"\xc0\x0c\0\x01\0\x01\0\0\0\x3c\0\x04" + answer
    s.sendto(reply, peer)
PY
ip netns exec "$(ns srv)" python3 "$WORK/dns.py" "$EXTERNAL_ADDR" &
echo $! >"$WORK/dns.pid"

snapshot() {
    {
        in_ns "$1" ip -4 rule
        in_ns "$1" ip -6 rule
        in_ns "$1" ip route show table all
        in_ns "$1" nft list ruleset
        in_ns "$1" sysctl -n net.ipv4.ip_forward
        cat "/etc/netns/$(ns "$1")/resolv.conf"
    } 2>&1 | grep -v 'fe80::' # the kernel adds link-local routes on its own schedule
}
for n in $NODES; do snapshot "$n" >"$WORK/$n/before"; done

start() {
    _sn=$1
    extra=${2:-}
    cat >"$WORK/$_sn/config.toml" <<CONF
[daemon]
state_dir = "$WORK/$_sn/state"
socket_path = "$WORK/$_sn/kith.sock"

[network]
listen_port = $PORT
$extra
CONF
    # Hide the host's systemd-resolved: it manages the host's links, not this namespace's.
    # Where /etc/resolv.conf links into its directory, point the link at this namespace's
    # file again. Background `ip` itself, not a function, so `$!` is the daemon.
    ip netns exec "$(ns "$_sn")" unshare --mount sh -c '
        if mount -t tmpfs none /run/systemd/resolve 2>/dev/null && [ -L /etc/resolv.conf ]; then
            target=$(readlink -f /etc/resolv.conf)
            touch "$target" && mount --bind "$2" "$target"
        fi
        exec "$0" daemon --config "$1" -v' \
        "$KITH" "$WORK/$_sn/config.toml" "/etc/netns/$(ns "$_sn")/resolv.conf" \
        >>"$WORK/$_sn/daemon.log" 2>&1 &
    echo $! >"$WORK/$_sn/pid"
    retry "$_sn daemon socket" 30 kith_in "$_sn" status
}

stop() {
    pid=$(cat "$WORK/$1/pid")
    kill -TERM "$pid"
    i=0
    while kill -0 "$pid" 2>/dev/null; do
        i=$((i + 1))
        [ "$i" -lt 50 ] || fail "$1 did not stop"
        sleep 0.2
    done
    rm -f "$WORK/$1/pid"
}

expect_clean() {
    snapshot "$1" >"$WORK/$1/after"
    diff -u "$WORK/$1/before" "$WORK/$1/after" || fail "$1 left changes behind"
}

SERVER="[server]
enabled = true
exit = true
relay = true
public_hostname = \"10.99.0.1\""

log "starting srv (exit and relay), creating the network"
start srv "$SERVER"
kith_in srv create --device-name srv "$NET" >/dev/null
invite() { kith_in srv invite "$NET" | grep -o 'kith://join/[A-Za-z0-9]*' | head -n 1; }

log "joining a and b"
for n in a b; do
    start "$n"
    kith_in "$n" join --device-name "$n" "$(invite)" >/dev/null
done

mesh_addr() {
    kith_in "$1" status --json |
        jq -r --arg d "$2" '.networks[].peers[] | select(.name == $d) | .address'
}
path_of() {
    kith_in "$1" status --json |
        jq -r --arg d "$2" '.networks[].peers[] | select(.name == $d) | "\(.path) \(.endpoint): \(.path_reason)"'
}
endpoint_of() {
    kith_in "$1" status --json |
        jq -r --arg d "$2" '.networks[].peers[] | select(.name == $d) | .endpoint'
}
known() { [ -n "$(mesh_addr "$1" "$2")" ]; }
relayed() { [ "$(endpoint_of "$1" "$2")" = relay ]; }
direct() {
    e=$(endpoint_of "$1" "$2")
    [ -n "$e" ] && [ "$e" != null ] && [ "$e" != relay ]
}
retry "b visible from a" 30 known a b
B=$(mesh_addr a b)
retry "ping a -> b over the mesh" 30 in_ns a ping -c 1 -W 1 "$B"
log "a reaches b at $B: $(path_of a b)"

log "names: mesh names resolve, and other names are forwarded to the LAN's resolver"
got=$(in_ns a dig +short +time=2 +tries=3 "@$MESH_DNS" "b.$NET.kith.internal" A)
[ "$got" = "$B" ] || fail "b.$NET.kith.internal resolved to '$got', want $B"
got=$(in_ns a dig +short +time=2 +tries=3 "@$MESH_DNS" "$EXTERNAL_NAME" A)
[ "$got" = "$EXTERNAL_ADDR" ] || fail "$EXTERNAL_NAME resolved to '$got' through kith, want $EXTERNAL_ADDR"
head -n 3 "/etc/netns/$(ns a)/resolv.conf" | grep -q "nameserver $MESH_DNS" ||
    fail "a's resolv.conf does not list the mesh resolver"

log "relay: blocking direct UDP between a and b"
in_ns a nft -f - <<'NFT'
table inet smoke {
    chain out { type filter hook output priority 0; ip daddr 10.99.0.3 udp dport 41641 drop; }
    chain in { type filter hook input priority 0; ip saddr 10.99.0.3 udp sport 41641 drop; }
}
NFT
retry "a -> b falls back to the relay" 40 relayed a b
retry "ping a -> b through the relay" 20 in_ns a ping -c 1 -W 1 "$B"
log "a reaches b via: $(path_of a b)"
in_ns a nft delete table inet smoke
retry "a -> b returns to direct" 60 direct a b
log "back to direct: $(path_of a b)"

log "exit: a sends internet traffic through srv"
in_ns a ping -c 1 -W 1 203.0.113.1 >/dev/null 2>&1 && fail "a reached the internet without an exit"
retry "srv granted as exit" 20 kith_in a exit use "$NET" srv
retry "ping a -> internet through srv" 20 in_ns a ping -c 1 -W 1 203.0.113.1
in_ns srv nft list chain ip filter DOCKER-USER | grep -q 'comment "kith"' ||
    fail "srv did not open DOCKER-USER for exit traffic"
kith_in a exit off >/dev/null
in_ns a ping -c 1 -W 1 203.0.113.1 >/dev/null 2>&1 && fail "a still reaches the internet after exit off"
in_ns a ip -4 rule | grep -q 27497 && fail "exit off left a's routing rules behind"

log "doctor on a"
kith_in a doctor || fail "kith doctor found problems on a healthy node"

log "restart: a stops cleanly and reconnects from its saved state"
stop a
expect_clean a
start a
retry "ping a -> b after a restart" 15 in_ns a ping -c 1 -W 1 "$B"

log "footprint"
for n in $NODES; do
    pid=$(cat "$WORK/$n/pid")
    child=$(pgrep -P "$pid" || echo "$pid")
    rss=$(awk '$1 == "VmRSS:" { print $2 }' "/proc/$child/status" 2>/dev/null || echo 0)
    printf '%s: VmRSS %s kB\n' "$n" "$rss"
    [ "$rss" -le $((32 * 1024)) ] || fail "$n uses ${rss} kB"
done
printf 'kith binary: %s bytes\n' "$(wc -c <"$KITH" | tr -d ' ')"

log "stopping every node; each namespace must be as it was"
for n in b a srv; do
    stop "$n"
    expect_clean "$n"
done
kill "$(cat "$WORK/dns.pid")" 2>/dev/null || true

log "smoke test passed"
