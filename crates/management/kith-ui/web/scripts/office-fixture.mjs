// Writes fixtures/office.json: a day in the life of the network "home", in the exact
// shape kith-ui sends (see the Fixture type in src/lib/types.gen.ts). Deterministic, so
// the file only changes when this script does. `cargo test -p kith-ui-core` checks the
// output against the daemon's real types.

import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const RECORDED = Date.UTC(2026, 10, 2, 14, 10, 0);
const DAY_START = RECORDED - 24 * HOUR;
const SAMPLE = 2 * SECOND;
const SAMPLES = 450;

let seed = 0x6b697468;
function random() {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
function hex(bytes) {
  let out = '';
  for (let i = 0; i < bytes; i += 1) out += Math.floor(random() * 256).toString(16).padStart(2, '0');
  return out;
}

const NET = hex(32);
const devices = [
  { name: 'laptop', address: '100.64.0.3', self: true },
  { name: 'desk', address: '100.64.0.2', path: 'direct', rtt: 3, endpoint: '192.168.1.12:41641', reason: 'LAN address answered first', rate: 180_000 },
  { name: 'nas', address: '100.64.0.4', path: 'direct', rtt: 3, endpoint: '192.168.1.20:41641', reason: 'LAN address answered first', rate: 1_200_000 },
  { name: 'vps', address: '100.64.0.1', path: 'direct', rtt: 34, endpoint: '203.0.113.40:41641', reason: 'Public address answered first', rate: 3_100_000 },
  { name: 'studio-mac', address: '100.64.0.5', path: 'direct', rtt: 12, endpoint: '198.51.100.23:52011', reason: 'Hole punched through both routers', rate: 64_000 },
  { name: 'dev-box', address: '100.64.0.6', path: 'relay', rtt: 84, endpoint: 'relay', reason: "dev-box's router changes ports for every connection, so no direct path exists", rate: 420_000, gap: true },
  { name: 'phone', address: '100.64.0.7', path: 'direct', rtt: 38, endpoint: '100.86.14.7:60211', reason: 'Connects through laptop', gateway: 'laptop', rate: 90_000 },
  { name: 'ipad', address: '100.64.0.8', path: 'direct', rtt: 9, endpoint: '192.168.1.31:51820', reason: 'Connects through desk', gateway: 'desk', rate: 12_000 },
  { name: 'tv', address: '100.64.0.9', offline: true, seen: 3 * HOUR + 12 * MINUTE },
  { name: 'office-pi', address: '100.64.0.10', path: 'stealth', rtt: 142, endpoint: 'relay', reason: 'UDP is blocked on its network, so traffic goes through vps over HTTPS', mtu: 1280, rate: 2_400 },
  { name: 'work-laptop', address: '100.64.0.11', path: 'direct', rtt: 162, endpoint: '192.0.2.77:41641', reason: 'Public address answered first', rate: 310_000, loss: 0.031 },
  { name: 'pi-hole', address: '100.64.0.12', path: 'direct', rtt: 4, endpoint: '192.168.1.53:41641', reason: 'LAN address answered first', rate: 600 },
];
for (const device of devices) device.node = hex(32);
const byName = Object.fromEntries(devices.map((d) => [d.name, d]));
const self = byName.laptop;
const vps = byName.vps;
const others = devices.filter((d) => !d.self);

function series(device) {
  if (device.offline) return [];
  const samples = [];
  let rx = Math.floor(random() * 4e9);
  let tx = Math.floor(rx * 0.08);
  for (let i = 0; i < SAMPLES; i += 1) {
    const at = RECORDED - (SAMPLES - 1 - i) * SAMPLE;
    if (device.gap && i >= 200 && i < 215) continue;
    if (device.gap && i === 215) {
      rx = 0;
      tx = 0;
    }
    const wave = 0.6 + 0.4 * Math.sin(i / 23) + (random() < 0.04 ? 2.5 * random() : 0);
    const rate = Math.max(0, device.rate * wave * (0.7 + 0.6 * random()));
    rx += Math.floor((rate * SAMPLE) / SECOND);
    tx += Math.floor((rate * 0.07 * SAMPLE) / SECOND);
    samples.push({ at_ms: at, rx_bytes: rx, tx_bytes: tx });
  }
  return samples;
}

const traffic = others.map((device) => ({
  network: NET,
  node: device.node,
  name: device.name,
  samples: series(device),
}));
const last = (device) => traffic.find((s) => s.node === device.node)?.samples.at(-1);

const peers = others.map((device) => ({
  node: device.node,
  name: device.name,
  address: device.address,
  path: device.offline ? null : device.path,
  path_reason: device.offline ? null : device.reason,
  last_seen_secs: device.offline ? Math.floor(device.seen / SECOND) : 1 + Math.floor(random() * 3),
  endpoint: device.offline ? null : device.endpoint,
  rx_bytes: last(device)?.rx_bytes ?? 0,
  tx_bytes: last(device)?.tx_bytes ?? 0,
  online: !device.offline,
  display_name: device.name,
  gateway: device.gateway ?? null,
  path_mtu: device.mtu ?? null,
}));

function link(from, to, rtt, loss, rate, age) {
  const via = to.path === 'relay' || to.path === 'stealth';
  return {
    network: NET,
    from: from.node,
    to: to.node,
    path: to.path,
    relay: via ? vps.node : null,
    rtt_ms: rtt,
    loss,
    rate,
    rate_band: null,
    age_secs: age,
  };
}
const links = others
  .filter((d) => !d.offline && !d.gateway)
  .map((d) => link(self, d, d.rtt, d.loss ?? 0, d.rate, 0));
links.push(
  link(byName.desk, byName.nas, 1, 0, 2_800_000, 3),
  link(byName.desk, byName.vps, 31, 0, 40_000, 3),
  link(byName.desk, { ...byName.ipad, path: 'direct' }, 2, 0, 12_000, 3),
  link(byName.vps, { ...byName['dev-box'], path: 'direct' }, 21, 0, 420_000, 5),
  link(byName.vps, { ...byName['office-pi'], path: 'direct' }, 96, 0.004, 2_400, 5),
  link(byName.nas, byName['pi-hole'], 1, 0, 300, 4),
  link(byName['studio-mac'], byName.vps, 40, 0, 64_000, 6),
);
links.push({ ...link(self, byName.phone, 38, 0, 90_000, 0) });

const entries = [];
function push(at, kind, fields) {
  entries.push({ seq: 0, at_ms: at, kind, ...fields });
}
const peer = (d) => ({ network: NET, node: d.node, name: d.name });
const log = (at, level, target, message) => push(at, 'log', { level, target, message });

log(DAY_START + 2 * MINUTE, 'info', 'kith_node::daemon', 'kith 0.1.1 started; listening on 41641');
for (const d of others) {
  push(DAY_START + 3 * MINUTE + Math.floor(random() * 40 * SECOND), 'peer_online', {
    ...peer(d),
    path: d.offline ? 'direct' : d.path,
    gateway: d.gateway ?? null,
  });
}
push(DAY_START + 3 * HOUR, 'exit_changed', { exit: 'home/vps' });
for (let quarter = 4; quarter < 24 * 4; quarter += 1) {
  const at = DAY_START + quarter * 15 * MINUTE + Math.floor(random() * 10 * MINUTE);
  log(at, 'info', 'kith_node::channel', `gossip: ${others.length} link reports, 0 rejected`);
  if (quarter % 2 === 0) log(at + 4 * SECOND, 'debug', 'kith_disco', 'refreshed STUN mapping 198.51.100.4:41641');
  if (quarter % 3 === 0) log(at + 9 * SECOND, 'trace', 'kith_wg', `rekeyed sessions for ${others.length - 1} peers`);
}
for (let h = 1; h < 22; h += 2) {
  const asleep = DAY_START + h * HOUR + Math.floor(random() * 30 * MINUTE);
  push(asleep, 'peer_offline', peer(byName.phone));
  push(asleep + 20 * MINUTE + Math.floor(random() * 20 * MINUTE), 'peer_online', {
    ...peer(byName.phone),
    path: 'direct',
    gateway: 'laptop',
  });
}
push(DAY_START + 10 * HOUR, 'peer_offline', peer(byName['studio-mac']));
push(DAY_START + 17 * HOUR + 40 * MINUTE, 'peer_online', { ...peer(byName['studio-mac']), path: 'direct', gateway: null });
push(DAY_START + 17 * HOUR + 41 * MINUTE, 'endpoint_changed', { ...peer(byName['studio-mac']), endpoint: '198.51.100.23:52011' });
for (let i = 0; i < 3; i += 1) {
  push(DAY_START + (11 + i * 3) * HOUR, 'dropped', { reason: 'policy', count: 4 + i * 3 });
}
push(DAY_START + 6 * HOUR, 'device_removed', { network: NET, node: hex(32), name: 'old-tablet' });
push(DAY_START + 7 * HOUR + 12 * MINUTE, 'device_joined', { ...peer(byName['work-laptop']), address: byName['work-laptop'].address });
push(DAY_START + 7 * HOUR + 13 * MINUTE, 'peer_online', { ...peer(byName['work-laptop']), path: 'direct', gateway: null });
push(DAY_START + 9 * HOUR, 'warning', { message: 'Another VPN holds a route to 100.64.0.0/10; kith routes take priority' });
log(DAY_START + 9 * HOUR + 1 * SECOND, 'warn', 'kith_sys::route', 'route 100.64.0.0/10 via utun4 conflicts with kith0');
push(RECORDED - 3 * HOUR - 12 * MINUTE, 'peer_offline', peer(byName.tv));
for (let i = 0; i < 6; i += 1) {
  const at = RECORDED - 68 * MINUTE + i * 3 * MINUTE;
  push(at, 'path_changed', { ...peer(byName['dev-box']), from: i % 2 ? 'direct' : 'relay', to: i % 2 ? 'relay' : 'direct' });
}
push(RECORDED - 50 * MINUTE, 'endpoint_changed', { ...peer(byName.phone), endpoint: '100.86.14.7:60211' });
log(RECORDED - 41 * MINUTE, 'error', 'kith_transport_quic', 'handshake with 192.0.2.77:41641 timed out after 5 s');
push(RECORDED - 40 * MINUTE, 'path_changed', { ...peer(byName['office-pi']), from: 'relay', to: 'stealth' });
push(RECORDED - 22 * MINUTE, 'dropped', { reason: 'policy', count: 12 });
for (let i = 0; i < 4; i += 1) {
  push(RECORDED - 9 * MINUTE + i * 2 * MINUTE, 'blocked', {
    network: NET,
    from: byName.phone.node,
    from_name: 'phone',
    to: byName.nas.node,
    to_name: 'nas',
    protocol: 'tcp',
    port: 22,
    count: 1,
  });
}
const pixel = hex(32);
push(RECORDED - 2 * MINUTE, 'join_requested', { network: NET, node: pixel, name: 'pixel-8' });
log(RECORDED - 1 * MINUTE, 'info', 'kith_node::daemon', 'link report from vps: 4 links');
entries.sort((a, b) => a.at_ms - b.at_ms);
entries.forEach((entry, index) => {
  entry.seq = index;
});

const name = (d) => `${d.name}.home.kith.internal`;
const records = [
  ...devices.map((d) => ({ name: name(d), type: 'A', value: d.address, device: true, set_by: null })),
  { name: 'files.home.kith.internal', type: 'CNAME', value: 'nas.home.kith.internal', device: false, set_by: 'laptop' },
  { name: 'git.home.kith.internal', type: 'A', value: '100.64.0.6', device: false, set_by: 'desk' },
  { name: 'backup.home.kith.internal', type: 'TXT', value: 'nightly 02:00 to nas', device: false, set_by: 'desk' },
];

const status = {
  node: self.node,
  networks: [
    { id: NET, name: 'home', peers_online: 10, peers_total: 11, relays: 1, problems: [], admin: true },
  ],
  exit: false,
  relay: false,
  exit_node: 'home/vps',
  listen_port: 41641,
  dns_upstreams: ['1.1.1.1', '9.9.9.9'],
  drops: { no_route: 2, policy: 37 },
  route_conflicts: [],
  warnings: [{ secs_ago: 15 * HOUR / SECOND, message: 'Another VPN holds a route to 100.64.0.0/10; kith routes take priority' }],
  imported: [
    {
      network: '0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1ce0f1c',
      name: 'work-vpn',
      endpoint: '198.51.100.20:51820',
      routes: ['10.20.0.0/16'],
      everything: false,
      active: true,
      last_seen_secs: 8,
      rx_bytes: 18_400_000,
      tx_bytes: 2_100_000,
    },
  ],
  operator: true,
  versions: { ui: '0.1.1', daemon: '0.1.1', compatible: true },
  paused_until: null,
  memberships: [
    {
      network: NET,
      name: 'laptop',
      address: self.address,
      role: 'admin',
      key_fpr: 'k7f3·9a21',
      made_admin_by: 'desk',
      made_admin_at_ms: Date.UTC(2026, 9, 12, 9, 30, 0),
      admins: ['desk', 'laptop'],
      share_links: true,
    },
  ],
  host: { name: 'laptop', os: 'macOS' },
  exit_state: 'active',
};

// The policy as the daemon writes it: everyone reaches vps, only desk and laptop reach
// everything, so phone's ssh to nas is blocked.
const device = (d) => `{ Device = "${d.node}" }`;
const everything = 'protocol = "Any"\nports = "All"\n';
const policy = `[[rules]]
sources = ["Any"]

[[rules.destinations]]
selector = ${device(vps)}
${everything}
[[rules]]
sources = [${device(self)}, ${device(byName.desk)}]

[[rules.destinations]]
selector = "Any"
${everything}`;

const fixture = {
  recorded_at_ms: RECORDED,
  boot: '9f2c1a7e5b3d4c60',
  status,
  peers: { [NET]: peers },
  records: { [NET]: records },
  pending: {
    joins: [
      {
        id: `join:${NET}:${pixel}`,
        network: NET,
        node: pixel,
        check_code: 'K7Q4XM',
        claimed: { name: 'pixel-8', os: 'Android' },
        seen: { ip: '203.0.113.7', key_fpr: 'k2c1·77f0' },
        requested_at_ms: RECORDED - 2 * MINUTE,
        expires_at_ms: RECORDED + 8 * MINUTE,
      },
    ],
    blocked: [
      {
        id: `blocked:${NET}:${byName.phone.node}:${byName.nas.node}:tcp:22`,
        network: NET,
        from: byName.phone.node,
        from_name: 'phone',
        to: byName.nas.node,
        to_name: 'nas',
        protocol: 'tcp',
        port: 22,
        count: 4,
        first_ms: RECORDED - 9 * MINUTE,
        last_ms: RECORDED - 3 * MINUTE,
        rule: '[[allow]]\nfrom = "phone"\nto = "nas"\nports = ["tcp/22"]\n',
      },
    ],
  },
  journal: entries,
  traffic,
  links: { [NET]: links },
  exits: {
    [NET]: [
      { network: NET, node: vps.node, name: 'vps', allowed: true, access: [], in_use_by: ['laptop', 'phone'] },
      { network: NET, node: byName.nas.node, name: 'nas', allowed: true, access: ['desk', 'laptop'], in_use_by: [] },
      { network: NET, node: byName['work-laptop'].node, name: 'work-laptop', allowed: false, access: ['desk'], in_use_by: [] },
    ],
  },
  invites: {
    [NET]: [
      {
        id: hex(32),
        expires_at_ms: RECORDED + 40 * MINUTE,
        uses_left: 1,
        created_at_ms: RECORDED - 20 * MINUTE,
      },
    ],
  },
  policies: { [NET]: policy },
  relays: { [NET]: [{ name: 'vps', address: '203.0.113.40:443', node: vps.node }] },
};

const target = fileURLToPath(new URL('../fixtures/office.json', import.meta.url));
writeFileSync(target, `${JSON.stringify(fixture)}\n`);
