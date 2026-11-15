#!/usr/bin/env bash
# Builds the page and the kith-ui binary, then serves a simulated Console on 41695.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
web="$here/.."
repo="$(cd "$web/../../../.." && pwd)"
port=41695
url_file="$here/.setup-url"
rm -f "$url_file"

(cd "$web" && npx vite build >/dev/null)
(cd "$repo" && cargo build -q -p kith-ui --target-dir target-owner)

state="$(mktemp -d)"
cp "$web/../web/fixtures/office.json" "$state/office.json"
log="$state/kith-ui.log"
"$repo/target-owner/debug/kith-ui" --fixture="$state/office.json" --fixture-console --no-open --port=$port >"$log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null; rm -rf "$state"' EXIT INT TERM

for _ in $(seq 1 100); do
  link="$(grep -oE "http://localhost:$port/setup/[A-Za-z0-9_-]+" "$log" | head -1 || true)"
  if [ -n "$link" ]; then echo "$link" >"$url_file"; break; fi
  kill -0 $pid 2>/dev/null || { cat "$log"; exit 1; }
  sleep 0.2
done
[ -s "$url_file" ] || { echo "no setup link printed"; cat "$log"; exit 1; }
wait $pid
