import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { copy, copyScreens } from '../copy';
import { clock, weekdayClock } from '../format';
import { JOURNAL_CAP } from '../live/timing';
import type { Entry, Fixture } from '../types.gen';
import {
  buildFeed,
  describeGroup,
  describe as say,
  gapText,
  logLine,
  olderEntries,
  rangeFor,
  redact,
  showsUnder,
  type Context,
  type Filter,
} from './activity';

const NET = 'n1';
const ctx: Context = {
  selfName: () => 'laptop',
  networkName: () => 'home',
  relayName: () => 'vps',
};
const all: Filter = { network: NET, kind: null, device: null, query: '' };

const NOW = new Date(2026, 10, 2, 15, 0, 0).getTime();
const MIN = 60_000;

let seq = 0;
function at(ms: number, kind: Record<string, unknown>): Entry {
  seq += 1;
  return { seq, at_ms: ms, ...kind } as Entry;
}
const blocked = (ms: number, count = 1): Entry =>
  at(ms, {
    kind: 'blocked',
    network: NET,
    from: 'p',
    from_name: 'phone',
    to: 'n',
    to_name: 'nas',
    protocol: 'tcp',
    port: 22,
    count,
  });
const flap = (ms: number): Entry =>
  at(ms, { kind: 'path_changed', network: NET, node: 'l', name: 'laptop', from: 'direct', to: 'relay' });
const log = (ms: number, level: string): Entry =>
  at(ms, { kind: 'log', level, target: 'kith_node::daemon', message: `a ${level} line` });

describe('older entries from the Console', () => {
  it('keeps what the live journal lacks, oldest first, numbered below every seq so far', () => {
    const live = [log(NOW, 'warn'), log(NOW + MIN, 'warn')];
    const first = live[0]?.seq ?? 0;
    const previousRun = log(NOW - 9 * MIN, 'warn');
    const kept = [
      { id: 9, boot: 'b2', entry: live[1] as Entry },
      { id: 8, boot: 'b2', entry: live[0] as Entry },
      { id: 7, boot: 'b2', entry: { ...log(NOW - MIN, 'warn'), seq: first - 1 } },
      { id: 6, boot: 'b1', entry: { ...previousRun, seq: first + 5 } },
    ];
    const older = olderEntries(kept, live, 'b2', -2);
    expect(older.map((e) => [e.seq, e.at_ms])).toEqual([
      [-4, NOW - 9 * MIN],
      [-3, NOW - MIN],
    ]);
    const feed = buildFeed(live, [...older, ...live], { ...all, network: null }, ctx, [], NOW);
    const entries = feed.flatMap((row) => (row.type === 'entry' ? [row.entry.at_ms] : []));
    expect(entries).toEqual([NOW + MIN, NOW, NOW - MIN, NOW - 9 * MIN]);
  });
});

describe('activity templates (spec §12.3)', () => {
  it('words each kind of entry', () => {
    const t = NOW;
    expect(
      say(at(t, { kind: 'peer_online', network: NET, node: 'p', name: 'phone', path: 'direct', gateway: null }), ctx)
        .sentence,
    ).toBe('phone came online · direct');
    expect(say(at(t, { kind: 'peer_offline', network: NET, node: 't', name: 'tv' }), ctx).sentence).toBe(
      'tv went offline',
    );
    expect(
      say(at(t, { kind: 'path_changed', network: NET, node: 'p', name: 'phone', from: 'direct', to: 'relay' }), ctx)
        .sentence,
    ).toBe('phone now relayed through vps (was direct)');
    expect(
      say(at(t, { kind: 'endpoint_changed', network: NET, node: 'p', name: 'phone', endpoint: 'x' }), ctx).sentence,
    ).toBe('phone changed network');
    const joined = say(
      at(t, { kind: 'device_joined', network: NET, node: 'x', name: 'pixel-8', address: '100.64.0.9' }),
      ctx,
    );
    expect(joined).toMatchObject({ sentence: 'pixel-8 joined', glyph: 'seal' });
    expect(say(at(t, { kind: 'exit_changed', exit: 'home/vps' }), ctx).sentence).toBe(
      'This device now uses vps as exit',
    );
    const refused = say(blocked(t, 4), ctx);
    expect(refused).toMatchObject({
      sentence: 'Blocked phone → nas:22 · 4 times',
      glyph: 'ban',
      actor: 'reported by laptop',
    });
    expect(say(at(t, { kind: 'warning', message: 'Route clash' }), ctx)).toMatchObject({
      sentence: 'Route clash',
      glyph: 'triangle-alert',
    });
  });

  it('hides info, debug and trace lines outside Logs', () => {
    const lines = ['error', 'warn', 'info', 'debug', 'trace'].map((level) => log(NOW, level));
    expect(lines.filter((e) => showsUnder(e, null)).map((e) => (e.kind === 'log' ? e.level : ''))).toEqual([
      'error',
      'warn',
    ]);
    expect(lines.every((e) => showsUnder(e, 'logs'))).toBe(true);
    expect(lines.some((e) => showsUnder(e, 'changes'))).toBe(false);
  });
});

describe('activity feed (spec §6.5)', () => {
  it('groups blocked connections within 10 minutes and sums their counts', () => {
    const journal = [blocked(NOW - 30 * MIN, 2), blocked(NOW - 25 * MIN, 3), blocked(NOW - 5 * MIN)];
    const rows = buildFeed(journal, journal, all, ctx, [], NOW);
    expect(rows.map((r) => r.type)).toEqual(['day', 'entry', 'group']);
    const group = rows[2];
    expect(group?.type === 'group' && group.entries.map((e) => e.seq)).toEqual([
      journal[1]?.seq,
      journal[0]?.seq,
    ]);
  });

  it('turns more than 3 path changes a minute apart into one flap row', () => {
    const few = [flap(NOW - 3 * MIN), flap(NOW - 2.5 * MIN), flap(NOW - 2 * MIN)];
    expect(buildFeed(few, few, all, ctx, [], NOW).filter((r) => r.type === 'entry')).toHaveLength(3);
    const many = [...few, flap(NOW - 1.5 * MIN)];
    const rows = buildFeed(many, many, all, ctx, [], NOW);
    expect(rows.map((r) => r.type)).toEqual(['day', 'group']);
  });

  it('groups the fixture dev-box flap, six changes three minutes apart', () => {
    const fixture = JSON.parse(
      readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
    ) as Fixture;
    const filter = { ...all, network: fixture.status.networks[0]?.id ?? null, kind: 'paths' as const };
    const last = fixture.journal[fixture.journal.length - 1]?.at_ms ?? 0;
    const rows = buildFeed(fixture.journal, fixture.journal, filter, ctx, [], last);
    const groups = rows.flatMap((r) => (r.type === 'group' ? [r] : []));
    expect(groups).toHaveLength(1);
    const sentence = describeGroup(groups[0]?.entries ?? [], ctx)?.sentence ?? '';
    expect(sentence).toMatch(/^dev-box switched paths 6 times since \d\d:\d\d$/);
  });

  it('divides days and marks gaps between the entries around them', () => {
    const yesterday = new Date(2026, 10, 1, 23, 50).getTime();
    const older = new Date(2026, 9, 31, 9, 0).getTime();
    const a = log(older, 'error');
    const b = log(yesterday, 'error');
    seq += 5;
    const c = log(NOW - MIN, 'error');
    const gap = { from: b.seq + 1, to: c.seq - 1, fromMs: b.at_ms, toMs: c.at_ms, position: 2 };
    const rows = buildFeed([a, b, c], [a, b, c], all, ctx, [gap], NOW);
    expect(rows.map((r) => (r.type === 'day' ? r.label : r.type))).toEqual([
      copy.time.today,
      'entry',
      'gap',
      copy.time.yesterday,
      'entry',
      expect.stringMatching(/31/),
      'entry',
    ]);
  });

  it('says only the end of a gap it knows', () => {
    const first = log(NOW - MIN, 'error');
    const gap = { from: 1, to: first.seq - 1, fromMs: NOW, toMs: first.at_ms, position: 0 };
    expect(gapText(gap, [first])).toBe(copyScreens.activity.gapBefore(clock(first.at_ms)));
    const late = new Date(2026, 10, 1, 23, 50).getTime();
    const before = log(late, 'error');
    seq += 3;
    const after = log(NOW, 'error');
    const across = { from: before.seq + 1, to: after.seq - 1, fromMs: late, toMs: NOW, position: 1 };
    expect(gapText(across, [before, after])).toBe(
      copy.activity.gap(weekdayClock(late), weekdayClock(NOW)),
    );
  });

  it('ends a full feed with the cap row', () => {
    const journal = Array.from({ length: JOURNAL_CAP }, () => log(NOW - MIN, 'warn'));
    const rows = buildFeed(journal, journal.slice(-3), all, ctx, [], NOW);
    expect(rows[rows.length - 1]).toMatchObject({ type: 'cap', text: copy.activity.cap });
  });

  it('filters by device and search', () => {
    const journal = [blocked(NOW - MIN), log(NOW - MIN, 'error')];
    const byDevice = buildFeed(journal, journal, { ...all, device: 'nas' }, ctx, [], NOW);
    expect(byDevice.filter((r) => r.type === 'entry')).toHaveLength(1);
    const bySearch = buildFeed(journal, journal, { ...all, query: 'ERROR line' }, ctx, [], NOW);
    expect(bySearch.filter((r) => r.type === 'entry')).toHaveLength(1);
  });

  it('words the whole office fixture', () => {
    const fixture = JSON.parse(
      readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
    ) as Fixture;
    for (const entry of fixture.journal) expect(say(entry, ctx).sentence).not.toBe('');
    const last = fixture.journal[fixture.journal.length - 1]?.at_ms ?? 0;
    const filter = { ...all, network: fixture.status.networks[0]?.id ?? null, kind: 'logs' as const };
    const rows = buildFeed(fixture.journal, fixture.journal, filter, ctx, [], last);
    expect(rows.filter((r) => r.type === 'entry').length).toBeGreaterThan(100);
  });
});

describe('links and the redacted log', () => {
  it('picks the smallest range that holds the moment', () => {
    expect(rangeFor(NOW - 30_000, NOW)).toBe('1m');
    expect(rangeFor(NOW - 10 * MIN, NOW)).toBe('15m');
    expect(rangeFor(NOW - 50 * MIN, NOW)).toBe('1h');
    expect(rangeFor(NOW - 90 * MIN, NOW)).toBeNull();
  });

  it('shortens addresses and keys but keeps times', () => {
    expect(redact('from 81.2.69.160:51820 at 14:02:16')).toBe('from 81.2.x.x:51820 at 14:02:16');
    expect(redact('peer 2001:db8::1 up')).toBe('peer 2001:db8:x:x up');
    expect(redact('key yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk= ok')).toBe('key yAnz… ok');
    expect(redact('node e8d09da6dc7b32f1f1916147a53fa63b gone')).toBe('node e8d0… gone');
    const line = logLine(log(Date.UTC(2026, 10, 2, 14, 2, 16), 'info'));
    expect(line).toBe('2026-11-02T14:02:16.000Z INF kith_node::daemon: a info line');
  });
});

describe('copyScreens (spec §12.1, §12.2)', () => {
  function strings(value: unknown, out: string[]): string[] {
    if (typeof value === 'string') out.push(value);
    else if (typeof value === 'function') {
      for (const args of [['nas', 'home', 'vps', 'x'], [2, 3, 4, 5], [['desk', 'nas']]]) {
        try {
          strings((value as (...a: unknown[]) => unknown)(...args), out);
        } catch {
          // Another sample fits this template.
        }
      }
    } else if (value && typeof value === 'object') {
      for (const v of Object.values(value)) strings(v, out);
    }
    return out;
  }

  it('keeps the banned words and the old vocabulary out', () => {
    const banned = [/!/, /successfully/i, /\bplease\b/i, /\boops\b/i, /Error:/, /Loading…/];
    const never = [/\bpeers?\b/i, /\bmesh\b/i, /\bexit node\b/i, /\bjournal\b/i, /\bdropped\b/i];
    for (const text of strings(copyScreens, [])) {
      for (const pattern of [...banned, ...never]) expect(text).not.toMatch(pattern);
    }
  });
});
