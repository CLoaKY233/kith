# kith server on AWS Lightsail

`up.sh` makes a kith exit node and relay: a `nano_3_1` instance ($5 a month, Mumbai by default) with a static IP, SSH open to one address only, and kith's ports (41641 UDP and TCP, 41642 TCP for the relay). `down.sh` removes exactly what `up.sh` made.

```sh
infra/lightsail/up.sh                                   # latest release, new identity
RESTORE=notes/kith-server-state.tar.gz infra/lightsail/up.sh   # bring back a saved server
infra/lightsail/down.sh --backup notes/kith-server-state.tar.gz
```

A restored state brings back the server's identity, so it is the same device in the same networks, with the same admin rights. The backup holds private keys: keep it out of git and off shared disks.

Set `KITH_BINARY` (and `KITH_UI_BINARY`) to deploy a local build instead of the latest release, and `CF_API_TOKEN`, `CF_ZONE_ID` and `DNS_NAME` to point a Cloudflare A record at the server. Both scripts need the AWS CLI signed in.
