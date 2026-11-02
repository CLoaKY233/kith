// Command palette matching (spec §10): `>` limits to actions, `@` to devices, `name:port`
// finds a device, and results rank by recency, then by how well they match.

import { copy } from '../copy';

export type Scope = 'all' | 'actions' | 'devices';

export type Query = {
  readonly scope: Scope;
  /** The text to match, without the prefix or the `:port`. */
  readonly text: string;
  /** The port of a `name:port` query, when there is one. */
  readonly port?: number;
};

const PORT = /^(.+):(\d{1,5})$/;
const MAX_PORT = 65_535;

/** Splits what was typed into a scope, the text to match and an optional port. */
export function parseQuery(raw: string): Query {
  let rest = raw.trimStart();
  let scope: Scope = 'all';
  if (rest.startsWith(copy.palette.prefix.actions)) {
    scope = 'actions';
    rest = rest.slice(copy.palette.prefix.actions.length);
  } else if (rest.startsWith(copy.palette.prefix.devices)) {
    scope = 'devices';
    rest = rest.slice(copy.palette.prefix.devices.length);
  }
  const text = rest.trim();
  const match = scope === 'actions' ? null : PORT.exec(text);
  const port = match ? Number(match[2]) : NaN;
  if (match?.[1] && port >= 1 && port <= MAX_PORT) {
    return { scope, text: match[1].trim(), port };
  }
  return { scope, text };
}

const WORD_START = /[\s\-._:/@]/;

/** Letters in order count only for a query this long, so two letters never match noise. */
const LETTERS_MIN = 3;

/**
 * How well `query` matches `target`, from 0 (no match) to 1 (equal). Prefixes beat word
 * starts, which beat substrings, which beat letters in order. Letters in order must each
 * start a word or follow the previous match (`nas` finds `north-america-south`, not
 * `Rename vps`).
 */
export function score(target: string, query: string): number {
  const t = target.toLowerCase();
  const q = query.toLowerCase();
  if (q === '') return 1;
  if (t === q) return 1;
  if (t.startsWith(q)) return 0.9;
  const at = t.indexOf(q);
  if (at > 0) return WORD_START.test(t.charAt(at - 1)) ? 0.8 : 0.6;
  if (q.length < LETTERS_MIN) return 0;
  let from = 0;
  let last = -2;
  for (const char of q) {
    let found = -1;
    for (let i = t.indexOf(char, from); i >= 0; i = t.indexOf(char, i + 1)) {
      if (i === last + 1 || i === 0 || WORD_START.test(t.charAt(i - 1))) {
        found = i;
        break;
      }
    }
    if (found < 0) return 0;
    last = found;
    from = found + 1;
  }
  return 0.3 * (q.length / t.length);
}

/** Below this a match is letters in order: recency does not lift it over real matches. */
const STRONG = 0.5;

export type Rankable = { readonly id: string; readonly label: string; readonly keywords?: readonly string[] };

/** A keyword match counts slightly less than the same match on the label. */
const KEYWORD_WEIGHT = 0.95;

export type Ranked<T> = { readonly item: T; readonly index: number; readonly best: number; readonly seen: number };

const strong = (best: number): number => (best >= STRONG ? 0 : 1);

/**
 * Real matches before letters in order; within each, recent first (most recent first),
 * then better matches, then the caller's order.
 */
export function compareRanked<T>(a: Ranked<T>, b: Ranked<T>): number {
  return (
    strong(a.best) - strong(b.best) || a.seen - b.seen || b.best - a.best || a.index - b.index
  );
}

/**
 * The items that match `text`: prefix, word and substring matches before letters in order;
 * within each, recently picked ones first (most recent first), then by match quality, ties
 * kept in the caller's order.
 */
export function rank<T extends Rankable>(items: readonly T[], text: string, recent: readonly string[]): T[] {
  return rankScored(items, text, recent).map((entry) => entry.item);
}

/** `rank`, keeping each item's score and recency so groups can be ordered by their best. */
export function rankScored<T extends Rankable>(
  items: readonly T[],
  text: string,
  recent: readonly string[],
): Ranked<T>[] {
  const scored = items
    .map((item, index) => {
      const best = Math.max(
        score(item.label, text),
        ...(item.keywords ?? []).map((k) => score(k, text) * KEYWORD_WEIGHT),
      );
      const seen = recent.indexOf(item.id);
      return { item, index, best, seen: seen < 0 ? Number.POSITIVE_INFINITY : seen };
    })
    .filter((entry) => entry.best > 0);
  return scored.sort(compareRanked);
}

/** Puts `id` at the front of the recency list, keeping at most `max` ids. */
export function remember(recent: readonly string[], id: string, max: number): string[] {
  return [id, ...recent.filter((other) => other !== id)].slice(0, max);
}
