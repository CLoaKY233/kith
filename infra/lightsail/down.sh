#!/bin/sh
# Removes the kith server that up.sh made: instance, static IP and key pair. Nothing else.
#
#   infra/lightsail/down.sh [--backup path.tar.gz]
#
# --backup first saves the server's kith state (its identity, so its admin rights, and
# every network's ledger) so up.sh RESTORE=... can bring the same networks back.
# Settings: REGION=ap-south-1 NAME=kith KEY=kith-vpn (same as up.sh); KEEP_KEY=1 keeps the key pair.
set -eu

REGION="${REGION:-ap-south-1}"
NAME="${NAME:-kith}"
KEY="${KEY:-kith-vpn}"
PEM="$HOME/.ssh/$KEY.pem"
LS="aws lightsail --region $REGION"

if [ "${1:-}" = --backup ]; then
    out="${2:?usage: down.sh --backup path.tar.gz}"
    ip="$($LS get-static-ip --static-ip-name "$NAME-ip" --query staticIp.ipAddress --output text)"
    ssh -i "$PEM" "ubuntu@$ip" 'sudo tar -czf - -C /var/lib kith' > "$out"
    chmod 600 "$out"
    echo "saved $out (holds private keys; keep it safe)"
fi

$LS delete-instance --instance-name "$NAME" --force-delete-add-ons >/dev/null 2>&1 && echo "deleted instance $NAME" || true
$LS release-static-ip --static-ip-name "$NAME-ip" >/dev/null 2>&1 && echo "released static IP $NAME-ip" || true
if [ -z "${KEEP_KEY:-}" ]; then
    $LS delete-key-pair --key-pair-name "$KEY" >/dev/null 2>&1 && echo "deleted key pair $KEY" || true
fi
