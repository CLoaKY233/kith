import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Hold, JOURNAL_PAUSE_PX, JournalCursor, Paths, stableOrder } from './stability.svelte';
import { FLAP_COUNT, FLAP_WINDOW_MS, PATH_HYSTERESIS_MS, SORT_FREEZE_MS } from './timing';

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_800_000_000_000);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('paths', () => {
  it('settle a replayed change at once, and only a style change waits', () => {
    const styled: string[] = [];
    const paths = new Paths(() => Date.now(), (key) => styled.push(key));
    paths.seed('n/a', 'direct', Date.now());

    paths.observe('n/a', 'relay', Date.now() - PATH_HYSTERESIS_MS);
    expect(paths.shown.get('n/a')?.path).toBe('relay');
    expect(styled).toEqual(['n/a']);

    paths.observe('n/a', null, Date.now());
    expect(paths.shown.get('n/a')?.path).toBeNull();
    paths.observe('n/a', 'stealth', Date.now());
    expect(paths.shown.get('n/a')?.path).toBe('stealth');
    expect(styled).toEqual(['n/a']);
  });

  it('keep the old style when a change reverts within 5 s', () => {
    const styled: string[] = [];
    const paths = new Paths(() => Date.now(), (key) => styled.push(key));
    paths.seed('n/a', 'direct', Date.now());
    paths.observe('n/a', 'relay', Date.now());
    vi.advanceTimersByTime(PATH_HYSTERESIS_MS / 2);
    paths.observe('n/a', 'direct', Date.now());
    vi.advanceTimersByTime(PATH_HYSTERESIS_MS);
    expect(paths.shown.get('n/a')?.path).toBe('direct');
    expect(styled).toEqual([]);
  });

  it('read flapping after more than 3 changes in 60 s, until the window passes', () => {
    const paths = new Paths(() => Date.now(), () => undefined);
    paths.seed('n/a', 'direct', Date.now());
    const flaps = Array.from({ length: FLAP_COUNT }, (_, i) => Date.now() + i);
    paths.observe('n/a', 'relay', Date.now(), flaps);
    expect(paths.shown.get('n/a')?.flapping).toBe(false);
    paths.observe('n/a', 'direct', Date.now(), [Date.now()]);
    expect(paths.shown.get('n/a')?.flapping).toBe(true);
    vi.advanceTimersByTime(FLAP_WINDOW_MS);
    expect(paths.shown.get('n/a')?.flapping).toBe(false);
  });
});

describe('lists', () => {
  it('keep shown rows in place while held and put new rows last', () => {
    expect(stableOrder(['a', 'c'], ['a', 'b', 'c'], false)).toEqual(['a', 'b', 'c']);
    expect(stableOrder(['c', 'a', 'd'], ['a', 'b', 'c'], true)).toEqual(['c', 'a', 'b']);
  });

  it('stay held for 1 s after the pointer leaves, and while focus is inside', () => {
    const hold = new Hold();
    hold.pointerEnter();
    expect(hold.held).toBe(true);
    hold.pointerLeave();
    vi.advanceTimersByTime(SORT_FREEZE_MS - 1);
    expect(hold.held).toBe(true);
    vi.advanceTimersByTime(1);
    expect(hold.held).toBe(false);

    hold.focusIn();
    hold.pointerEnter();
    hold.pointerLeave();
    vi.advanceTimersByTime(SORT_FREEZE_MS);
    expect(hold.held).toBe(true);
    hold.focusOut();
    expect(hold.held).toBe(false);
  });
});

describe('journal cursor', () => {
  it('pauses past 48 px, counts what arrived since, and resumes', () => {
    const cursor = new JournalCursor();
    cursor.scrolled(JOURNAL_PAUSE_PX, 3);
    expect(cursor.paused).toBe(false);
    cursor.scrolled(JOURNAL_PAUSE_PX + 1, 3);
    expect(cursor.split(['a', 'b', 'c', 'd', 'e'], 5)).toEqual({ shown: ['a', 'b', 'c'], newer: ['d', 'e'] });
    // The cap dropped `a` and `b`; positions still line up.
    expect(cursor.split(['c', 'd', 'e', 'f'], 6)).toEqual({ shown: ['c'], newer: ['d', 'e', 'f'] });
    cursor.scrolled(0, 6);
    expect(cursor.paused).toBe(false);
    expect(cursor.split(['c', 'd'], 6).newer).toEqual([]);
  });
});
