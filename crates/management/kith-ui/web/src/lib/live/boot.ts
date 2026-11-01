// The boot snapshot (spec §2.5, §8.1). kith-ui inlines it into index.html as
// `<script type="application/json" id="boot">` so the first frame has complete data.
// Under `npm run dev` there is no inline copy and the store reads GET /api/boot instead.

import type { Boot } from '../types.gen';

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const isNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);

/** True when `value` has the shape of a Boot, checked as deep as the store relies on. */
export function isBoot(value: unknown): value is Boot {
  if (!isObject(value)) return false;
  const { boot, seq, server_time_ms, status, peers, traffic_tail, pending, versions } = value;
  if (typeof boot !== 'string' || boot === '') return false;
  if (!isNumber(seq) || seq < 0 || !isNumber(server_time_ms)) return false;
  if (!isObject(status) || !Array.isArray(status['networks']) || !Array.isArray(status['memberships'])) {
    return false;
  }
  if (!isObject(peers) || !Object.values(peers).every(Array.isArray)) return false;
  if (!Array.isArray(traffic_tail)) return false;
  if (!isObject(pending) || !Array.isArray(pending['joins']) || !Array.isArray(pending['blocked'])) {
    return false;
  }
  if (!isObject(versions) || typeof versions['compatible'] !== 'boolean') return false;
  const tick = value['last_tick'];
  return tick === null || tick === undefined || (isObject(tick) && isNumber(tick['at_ms']));
}

/** Parses the inline snapshot's text; `null` when it is missing or not a Boot. */
export function parseBoot(text: string | null | undefined): Boot | null {
  if (!text) return null;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isBoot(value)) return null;
  return { ...value, last_tick: value.last_tick ?? null };
}

/** Reads `#boot` from the page, when there is one. */
export function readInlineBoot(doc: Pick<Document, 'getElementById'> | null): Boot | null {
  return parseBoot(doc?.getElementById('boot')?.textContent);
}
