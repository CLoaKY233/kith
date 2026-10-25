#!/bin/sh
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
KITH="$ROOT/target/release/kith"
WORK=$(mktemp -d)
NS1=kith-smoke-1
NS2=kith-smoke-2
NETWORK=home
DEV1=dev1
DEV2=dev2
PORT=41641
RESOLVER=${KITH_SMOKE_RESOLVER:-100.64.0.1}
PIDS=""

log() { printf '==> %s\n' "$*"; }

dump_logs() {
    for n in 1 2; do
        if [ -f "$WORK/ns$n/daemon.log" ]; then
            printf '\n----- daemon ns%s log -----\n' "$n"
            cat "$WORK/ns$n/daemon.log"
        fi
    done
}

cleanup() {
    status=$?
    trap - EXIT INT TERM
    if [ "$status" -ne 0 ]; then
        printf '\nFAILED (exit %s)\n' "$status" >&2
        dump_logs >&2
    fi
    for pid in $PIDS; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in $PIDS; do
        wait "$pid" 2>/dev/null || true
    done
    ip netns del "$NS1" 2>/dev/null || true
    ip netns del "$NS2" 2>/dev/null || true
    rm -rf "$WORK"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

kith_in() {
    n=$1
    shift
    ip netns exec "kith-smoke-$n" env KITH_SOCKET="$WORK/ns$n/kith.sock" "$KITH" "$@"
}

wait_for() {
    what=$1
    shift
    i=0
    until "$@" >/dev/null 2>&1; do
        i=$((i + 1))
        [ "$i" -le 60 ] || fail "timed out waiting for $what"
        sleep 1
    done
}

[ "$(id -u)" -eq 0 ] || fail "run as root"
for tool in ip jq ping; do
    command -v "$tool" >/dev/null || fail "missing $tool"
done

log "building kith"
(cd "$ROOT" && cargo build -p kith-cli --release --locked)

log "creating namespaces"
ip netns add "$NS1"
ip netns add "$NS2"
ip link add kith-veth1 type veth peer name kith-veth2
ip link set kith-veth1 netns "$NS1"
ip link set kith-veth2 netns "$NS2"
ip -n "$NS1" addr add 10.99.0.1/24 dev kith-veth1
ip -n "$NS2" addr add 10.99.0.2/24 dev kith-veth2
for n in 1 2; do
    ip -n "kith-smoke-$n" link set lo up
    ip -n "kith-smoke-$n" link set "kith-veth$n" up
done
ip netns exec "$NS2" ping -c 1 -W 2 10.99.0.1 >/dev/null || fail "veth link down"

for n in 1 2; do
    dir="$WORK/ns$n"
    mkdir -p "$dir/state"
    cat >"$dir/config.toml" <<CONF
[daemon]
state_dir = "$dir/state"
socket_path = "$dir/kith.sock"

[network]
listen_port = $PORT
CONF
    log "starting daemon in ns$n"
    ip netns exec "kith-smoke-$n" "$KITH" daemon --config "$dir/config.toml" -v >"$dir/daemon.log" 2>&1 &
    PIDS="$PIDS $!"
    wait_for "daemon ns$n socket" kith_in "$n" status
done

log "creating network and invite in ns1"
kith_in 1 create --device-name "$DEV1" "$NETWORK"
LINK=$(kith_in 1 invite "$NETWORK" | tail -n 1)
[ -n "$LINK" ] || fail "empty invite link"

log "joining from ns2"
kith_in 2 join --device-name "$DEV2" "$LINK"

peer_online() {
    kith_in 2 status --json |
        jq -e --arg d "$DEV1" '.networks[].peers[] | select(.name == $d and .path != null)'
}
log "waiting for ns2 to see $DEV1"
wait_for "$DEV1 online in ns2" peer_online

ADDR=$(kith_in 2 status --json |
    jq -r --arg d "$DEV1" '.networks[].peers[] | select(.name == $d) | .address' |
    head -n 1 | cut -d/ -f1)
[ -n "$ADDR" ] || fail "no mesh address for $DEV1"
log "$DEV1 mesh address: $ADDR"

log "pinging $ADDR from ns2"
ip netns exec "$NS2" ping -c 3 -W 2 "$ADDR" || fail "ping over the mesh failed"

log "footprint"
RSS_LIMIT_KB=$((32 * 1024))
n=0
for pid in $PIDS; do
    n=$((n + 1))
    [ -r "/proc/$pid/status" ] || fail "daemon ns$n (pid $pid) not running"
    rss=$(awk '$1 == "VmRSS:" { print $2 }' "/proc/$pid/status")
    hwm=$(awk '$1 == "VmHWM:" { print $2 }' "/proc/$pid/status")
    printf 'daemon ns%s pid %s: VmRSS %s kB, VmHWM %s kB\n' "$n" "$pid" "$rss" "$hwm"
    [ "$rss" -le "$RSS_LIMIT_KB" ] || fail "daemon ns$n idle RSS ${rss} kB exceeds ${RSS_LIMIT_KB} kB"
done
printf 'kith binary: %s bytes\n' "$(wc -c <"$KITH" | tr -d ' ')"

FQDN="$DEV1.$NETWORK.kith.internal"
log "resolving $FQDN in ns2"
if command -v dig >/dev/null; then
    GOT=$(ip netns exec "$NS2" dig +short +time=2 +tries=3 "@$RESOLVER" "$FQDN" A)
else
    GOT=$(ip netns exec "$NS2" getent ahostsv4 "$FQDN" | awk 'NR == 1 { print $1 }')
fi
printf '%s\n' "$GOT" | grep -qxF "$ADDR" || fail "$FQDN resolved to '$GOT', want $ADDR"

log "smoke test passed"
