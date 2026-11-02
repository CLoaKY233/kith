import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Fixture, LinkTick } from '../types.gen';
import { layout } from './layout';
import {
  buildModel,
  EDGE_MAX,
  EDGE_MIN,
  edges,
  layoutInput,
  layoutKey,
  nodeLabel,
  nodeName,
  relayUsers,
  remember,
  resolveNetwork,
  tableRows,
  type MapModel,
} from './model';

const fixture = JSON.parse(
  readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
) as Fixture;
const net = fixture.status.networks[0]?.id ?? '';

function office(links: readonly LinkTick[] = fixture.links[net] ?? []): MapModel {
  const peers = Object.fromEntries((fixture.peers[net] ?? []).map((p) => [p.node, p]));
  const model = buildModel(fixture.status, peers, links, net);
  if (!model) throw new Error('fixture has no network');
  return model;
}

function idOf(model: MapModel, name: string): string {
  const device = [...model.devices.values()].find((d) => d.name === name);
  if (!device) throw new Error(`no ${name}`);
  return device.id;
}

describe('map model on the office fixture', () => {
  const model = office();
  const vps = idOf(model, 'vps');
  const desk = idOf(model, 'desk');

  it('finds the network by id or name, and every device', () => {
    expect(resolveNetwork(fixture.status, net)).toBe(net);
    expect(resolveNetwork(fixture.status, 'home')).toBe(net);
    expect(resolveNetwork(fixture.status, 'nowhere')).toBeNull();
    expect(model.devices.size).toBe(12);
    expect(model.devices.get(model.self)).toMatchObject({ name: 'laptop', self: true });
    expect(model.devices.get(idOf(model, 'ipad'))?.gateway).toBe(desk);
    expect(model.devices.get(idOf(model, 'phone'))?.gateway).toBe(model.self);
  });

  it('places vps on the relay ring, the ipad off desk and the phone on the ring', () => {
    const relays = remember(new Map(), relayUsers(model));
    expect([...(relays.get(vps) ?? [])].sort()).toEqual(
      [idOf(model, 'dev-box'), idOf(model, 'office-pi')].sort(),
    );
    const { nodes } = layout(layoutInput(model, relays), { radius: 200, satellite: 28, spacing: 20 });
    expect(nodes.get(vps)?.ring).toBe('relay');
    expect(nodes.get(idOf(model, 'ipad'))).toMatchObject({ ring: 'satellite', gateway: desk });
    expect(nodes.get(idOf(model, 'phone'))?.ring).toBe('outer');
  });

  it('remembers relays, so a path change does not move one', () => {
    const relays = remember(new Map(), relayUsers(model));
    const direct = office((fixture.links[net] ?? []).map((l) => ({ ...l, path: 'direct', relay: null })));
    expect(remember(relays, relayUsers(direct))).toBe(relays);
    const key = layoutKey(layoutInput(direct, relays), false);
    expect(layoutKey(layoutInput(model, relays), false)).toBe(key);
  });

  it('draws only this device’s links by default, relayed ones through the relay', () => {
    const drawn = edges(model, { showAll: false, focus: null });
    for (const e of drawn) {
      expect(e.strong).toBe(true);
      expect(e.labelled).toBe(false);
      if (e.kind !== 'gateway') expect(e.a).toBe(model.self);
    }
    const devBox = drawn.find((e) => e.b === idOf(model, 'dev-box'));
    expect(devBox).toMatchObject({ kind: 'relayed', via: vps });
    const ipad = drawn.find((e) => e.b === idOf(model, 'ipad'));
    expect(ipad).toMatchObject({ kind: 'gateway', a: desk, link: null });
    const phone = drawn.find((e) => e.b === idOf(model, 'phone'));
    expect(phone).toMatchObject({ kind: 'gateway', a: model.self });
    expect(drawn.find((e) => e.b === vps)?.width).toBe(EDGE_MAX);
    expect(drawn.find((e) => e.b === idOf(model, 'pi-hole'))?.width).toBe(EDGE_MIN);
  });

  it('turns a slow or lossy link amber and a silent online device red', () => {
    const drawn = edges(model, { showAll: false, focus: null });
    expect(drawn.find((e) => e.b === idOf(model, 'work-laptop'))?.tone).toBe('attn');
    expect(drawn.find((e) => e.b === desk)?.tone).toBe('neutral');
    const quiet = office();
    const nas = quiet.devices.get(idOf(quiet, 'nas'))?.peer;
    if (nas) nas.last_seen_secs = 200;
    expect(edges(quiet, { showAll: false, focus: null }).find((e) => e.b === nas?.node)?.tone).toBe('danger');
  });

  it('shows a focused device’s reported links at full strength and dims the rest', () => {
    const drawn = edges(model, { showAll: false, focus: desk });
    const strong = drawn.filter((e) => e.strong);
    expect(strong.every((e) => e.a === desk || e.b === desk)).toBe(true);
    expect(strong.map((e) => e.b).sort()).toEqual(
      [desk, idOf(model, 'nas'), vps, idOf(model, 'ipad')].sort(),
    );
    expect(drawn.some((e) => !e.strong)).toBe(true);
    expect(drawn.filter((e) => e.labelled).every((e) => e.strong)).toBe(true);
  });

  it('dims nothing when the focused device has no links', () => {
    const drawn = edges(model, { showAll: false, focus: idOf(model, 'tv') });
    expect(drawn.every((e) => e.strong && !e.labelled)).toBe(true);
  });

  it('draws every report once with Show all links', () => {
    const drawn = edges(model, { showAll: true, focus: null });
    expect(new Set(drawn.map((e) => e.key)).size).toBe(drawn.length);
    expect(drawn.some((e) => e.a === idOf(model, 'studio-mac') && e.b === vps)).toBe(true);
  });

  it('labels nodes and names them for screen readers', () => {
    const tv = model.devices.get(idOf(model, 'tv'));
    const self = model.devices.get(model.self);
    if (!tv || !self) throw new Error('fixture changed');
    expect(nodeLabel(tv, true)).toBe('tv · 3 h ago');
    expect(nodeLabel(self, true)).toBe('this Mac');
    expect(nodeLabel(self, false)).toBe('this device');
    const workLaptop = model.devices.get(idOf(model, 'work-laptop'));
    if (!workLaptop) throw new Error('fixture changed');
    expect(nodeName(model, workLaptop, false)).toBe('work-laptop, online, direct, 162 milliseconds, slow');
    expect(nodeName(model, workLaptop, true)).toBe('work-laptop, last known, direct');
  });

  it('lists every device in the table twin', () => {
    const rows = tableRows(model, false);
    expect(rows[0]?.from).toBe('laptop');
    const named = new Set(rows.flatMap((r) => [r.from, r.to]));
    for (const d of model.devices.values()) expect(named.has(d.name)).toBe(true);
    expect(rows.find((r) => r.to === 'tv')).toMatchObject({ path: 'offline', latency: '—' });
    expect(rows.find((r) => r.to === 'dev-box' && r.from === 'laptop')?.path).toBe('via vps');
    expect(rows.find((r) => r.to === 'office-pi' && r.from === 'laptop')?.path).toBe('stealth via vps');
  });
});
