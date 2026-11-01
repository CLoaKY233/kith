import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SPINNER_DELAY_MS, SPINNER_MIN_MS } from '../live/timing';
import { pendingGate } from './pending';

describe('pendingGate', () => {
  let shown: boolean[];
  beforeEach(() => {
    vi.useFakeTimers();
    shown = [];
  });
  afterEach(() => vi.useRealTimers());

  it('never shows the spinner for an action that settles within the delay', () => {
    const gate = pendingGate((s) => shown.push(s));
    gate.set(true);
    vi.advanceTimersByTime(SPINNER_DELAY_MS - 1);
    gate.set(false);
    vi.advanceTimersByTime(SPINNER_MIN_MS * 2);
    expect(shown).toEqual([]);
  });

  it('holds a shown spinner for the minimum, then hides it once', () => {
    const gate = pendingGate((s) => shown.push(s));
    gate.set(true);
    vi.advanceTimersByTime(SPINNER_DELAY_MS);
    expect(shown).toEqual([true]);

    vi.advanceTimersByTime(50);
    gate.set(false);
    vi.advanceTimersByTime(SPINNER_MIN_MS - 51);
    expect(shown).toEqual([true]);
    vi.advanceTimersByTime(1);
    expect(shown).toEqual([true, false]);
  });

  it('keeps the spinner without a flicker when pending resumes during the hold', () => {
    const gate = pendingGate((s) => shown.push(s));
    gate.set(true);
    vi.advanceTimersByTime(SPINNER_DELAY_MS);
    gate.set(false);
    gate.set(true);
    vi.advanceTimersByTime(SPINNER_MIN_MS * 2);
    expect(shown).toEqual([true]);
  });

  it('hides at once when the spinner has already been shown long enough', () => {
    const gate = pendingGate((s) => shown.push(s));
    gate.set(true);
    vi.advanceTimersByTime(SPINNER_DELAY_MS + SPINNER_MIN_MS);
    gate.set(false);
    expect(shown).toEqual([true, false]);
  });
});
