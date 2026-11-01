import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { announce, attachAnnouncer, resetAnnouncer, type Politeness } from './announce';
import { ANNOUNCER_GAP_MS, BLOCKED_ANNOUNCE_MS } from './live/timing';

let said: [Politeness, string][] = [];

beforeEach(() => {
  vi.useFakeTimers();
  said = [];
  attachAnnouncer((politeness, message) => said.push([politeness, message]));
});

afterEach(() => {
  resetAnnouncer();
  vi.useRealTimers();
});

describe('announcer (spec §11.1)', () => {
  it('speaks one polite message at a time, at least the gap apart', () => {
    announce('pixel-8 is asking to join home.');
    announce('Blocked phone → nas :22');
    expect(said).toEqual([['polite', 'pixel-8 is asking to join home.']]);
    vi.advanceTimersByTime(ANNOUNCER_GAP_MS - 1);
    expect(said).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(said[1]).toEqual(['polite', 'Blocked phone → nas :22']);
  });

  it('coalesces repeats, and changes the text when the same message comes back later', () => {
    announce('a');
    announce('a');
    vi.advanceTimersByTime(ANNOUNCER_GAP_MS);
    announce('a');
    expect(said.map(([, m]) => m)).toEqual(['a', 'a ']);
  });

  it('drops a keyed message announced too recently', () => {
    const key = { key: 'blocked:h:phone:nas:tcp:22', every: BLOCKED_ANNOUNCE_MS };
    announce('blocked once', key);
    vi.advanceTimersByTime(ANNOUNCER_GAP_MS);
    announce('blocked twice', key);
    vi.advanceTimersByTime(BLOCKED_ANNOUNCE_MS);
    announce('blocked later', key);
    expect(said.map(([, m]) => m)).toEqual(['blocked once', 'blocked later']);
  });

  it('sends assertive messages at once, past the polite queue', () => {
    announce('first');
    announce('second');
    announce('kith stopped on this device.', 'assertive');
    expect(said).toEqual([
      ['polite', 'first'],
      ['assertive', 'kith stopped on this device.'],
    ]);
  });
});
