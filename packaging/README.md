# Packaging

## Linux (systemd)

```sh
cargo build --release -p kith-cli
sudo install -m 0755 target/release/kith /usr/bin/kith
sudo install -d -m 0755 /etc/kith
sudo install -m 0644 crates/management/kith-config/kith.example.toml /etc/kith/kith.toml
sudo install -m 0644 packaging/systemd/kith.service /etc/systemd/system/kith.service
sudo systemctl daemon-reload
sudo systemctl enable --now kith
journalctl -u kith -f
```

The unit runs the daemon with only `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE` and `CAP_NET_RAW`. State lives in `/var/lib/kith`, the socket in `/run/kith`.

## macOS (launchd)

```sh
cargo build --release -p kith-cli
sudo install -m 0755 target/release/kith /usr/local/bin/kith
sudo mkdir -p "/Library/Application Support/kith"
sudo cp crates/management/kith-config/kith.example.toml "/Library/Application Support/kith/kith.toml"
sudo install -m 0644 -o root -g wheel packaging/launchd/dev.kith.daemon.plist /Library/LaunchDaemons/dev.kith.daemon.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/dev.kith.daemon.plist
sudo launchctl print system/dev.kith.daemon
```

Stop with `sudo launchctl bootout system/dev.kith.daemon`. Logs go to `/var/log/kith.log`.

## OpenWrt

Build static musl binaries, one directory per target triple:

```sh
for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf; do
  cross build --release -p kith-cli --target "$t"
  mkdir -p dist/kith-0.1.0/"$t"
  cp target/"$t"/release/kith dist/kith-0.1.0/"$t"/
done
```

Then, in an OpenWrt SDK:

```sh
cp -r /path/to/kith/packaging/openwrt/kith package/kith
make package/kith/compile KITH_BIN_DIR=/path/to/kith/dist/kith-0.1.0 V=s
scp -O bin/packages/*/base/kith_*.ipk root@192.168.1.1:/tmp/
ssh root@192.168.1.1 'opkg install /tmp/kith_*.ipk && /etc/init.d/kith enable && /etc/init.d/kith start'
```

Supported `ARCH` values: `x86_64`, `aarch64`, `arm` (armv7 hard-float), `mipsel`, `mips`, `i386`. The config is `/etc/kith/kith.toml` and is kept across upgrades; edits restart the daemon via procd.
