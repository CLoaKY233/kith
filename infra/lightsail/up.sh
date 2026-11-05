#!/bin/sh
# Creates a kith server on AWS Lightsail: one nano instance with a static IP, a firewall
# that allows SSH from one address only, and kith running as an exit node and relay.
#
#   infra/lightsail/up.sh
#
# Settings (environment):
#   REGION=ap-south-1  ZONE=ap-south-1a  NAME=kith  BUNDLE=nano_3_1
#   KEY=kith-vpn            Lightsail key pair; made and saved to ~/.ssh/$KEY.pem if missing
#   SSH_FROM=<ip>/32        who may SSH in; defaults to this machine's public IPv4
#   KITH_BINARY=path        install this Linux x86-64 kith instead of the latest release
#   KITH_UI_BINARY=path     also install this kith-ui
#   RESTORE=path.tar.gz     restore a saved state (identity, networks) from down.sh --backup
#   CF_API_TOKEN, CF_ZONE_ID, DNS_NAME   point an A record at the server (Cloudflare)
set -eu

REGION="${REGION:-ap-south-1}"
ZONE="${ZONE:-ap-south-1a}"
NAME="${NAME:-kith}"
BUNDLE="${BUNDLE:-nano_3_1}"
KEY="${KEY:-kith-vpn}"
PEM="$HOME/.ssh/$KEY.pem"
SSH_FROM="${SSH_FROM:-$(curl -4 -fsS --max-time 10 https://checkip.amazonaws.com | tr -d '\n')/32}"
LS="aws lightsail --region $REGION"

say() { printf '%s\n' "$*"; }

if ! $LS get-key-pair --key-pair-name "$KEY" >/dev/null 2>&1; then
    say "creating key pair $KEY"
    $LS create-key-pair --key-pair-name "$KEY" --query privateKeyBase64 --output text > "$PEM"
    chmod 600 "$PEM"
fi
[ -f "$PEM" ] || { say "key pair $KEY exists but $PEM is missing"; exit 1; }

if ! $LS get-instance --instance-name "$NAME" >/dev/null 2>&1; then
    say "creating instance $NAME ($BUNDLE in $ZONE)"
    $LS create-instances --instance-names "$NAME" --availability-zone "$ZONE" \
        --blueprint-id ubuntu_24_04 --bundle-id "$BUNDLE" --key-pair-name "$KEY" \
        --ip-address-type dualstack >/dev/null
fi
until [ "$($LS get-instance-state --instance-name "$NAME" --query state.name --output text)" = running ]; do sleep 5; done

if ! $LS get-static-ip --static-ip-name "$NAME-ip" >/dev/null 2>&1; then
    $LS allocate-static-ip --static-ip-name "$NAME-ip" >/dev/null
fi
$LS attach-static-ip --static-ip-name "$NAME-ip" --instance-name "$NAME" >/dev/null 2>&1 || true
IP="$($LS get-static-ip --static-ip-name "$NAME-ip" --query staticIp.ipAddress --output text)"
say "static IP $IP"

$LS put-instance-public-ports --instance-name "$NAME" --port-infos "[
  {\"fromPort\":22,\"toPort\":22,\"protocol\":\"tcp\",\"cidrs\":[\"$SSH_FROM\"]},
  {\"fromPort\":41641,\"toPort\":41641,\"protocol\":\"udp\",\"cidrs\":[\"0.0.0.0/0\"],\"ipv6Cidrs\":[\"::/0\"]},
  {\"fromPort\":41641,\"toPort\":41642,\"protocol\":\"tcp\",\"cidrs\":[\"0.0.0.0/0\"],\"ipv6Cidrs\":[\"::/0\"]}
]" >/dev/null

SSH="ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -i $PEM ubuntu@$IP"
ssh-keygen -R "$IP" >/dev/null 2>&1 || true
until $SSH true 2>/dev/null; do sleep 5; done

# Small servers run better without these.
$SSH 'sudo systemctl disable --now ModemManager multipathd udisks2 >/dev/null 2>&1 || true'

if [ -n "${KITH_BINARY:-}" ]; then
    scp -q -i "$PEM" "$KITH_BINARY" "ubuntu@$IP:/tmp/kith"
    $SSH 'sudo install -m 0755 /tmp/kith /usr/local/bin/kith && rm /tmp/kith'
else
    $SSH 'curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | KITH_NO_START=1 sh'
fi
if [ -n "${KITH_UI_BINARY:-}" ]; then
    scp -q -i "$PEM" "$KITH_UI_BINARY" "ubuntu@$IP:/tmp/kith-ui"
    $SSH 'sudo install -m 0755 /tmp/kith-ui /usr/local/bin/kith-ui && rm /tmp/kith-ui'
fi

if [ -n "${RESTORE:-}" ]; then
    say "restoring state from $RESTORE"
    scp -q -i "$PEM" "$RESTORE" "ubuntu@$IP:/tmp/kith-state.tar.gz"
    $SSH 'sudo mkdir -p /var/lib /etc && sudo tar -xzf /tmp/kith-state.tar.gz -C /var/lib kith && rm /tmp/kith-state.tar.gz'
fi

$SSH 'sudo kith up && sudo kith server --exit --relay'

if [ -n "${CF_API_TOKEN:-}" ] && [ -n "${CF_ZONE_ID:-}" ] && [ -n "${DNS_NAME:-}" ]; then
    api="https://api.cloudflare.com/client/v4/zones/$CF_ZONE_ID/dns_records"
    id="$(curl -fsS -H "Authorization: Bearer $CF_API_TOKEN" "$api?type=A&name=$DNS_NAME" | sed -n 's/.*"id":"\([0-9a-f]\{32\}\)".*/\1/p' | head -1)"
    body="{\"type\":\"A\",\"name\":\"$DNS_NAME\",\"content\":\"$IP\",\"ttl\":60,\"proxied\":false}"
    if [ -n "$id" ]; then
        curl -fsS -X PUT -H "Authorization: Bearer $CF_API_TOKEN" -H 'Content-Type: application/json' "$api/$id" -d "$body" >/dev/null
    else
        curl -fsS -X POST -H "Authorization: Bearer $CF_API_TOKEN" -H 'Content-Type: application/json' "$api" -d "$body" >/dev/null
    fi
    say "$DNS_NAME -> $IP"
fi

$SSH 'kith status'
say "ssh: ssh -i $PEM ubuntu@$IP"
