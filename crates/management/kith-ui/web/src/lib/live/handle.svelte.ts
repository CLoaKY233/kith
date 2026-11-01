// One running action as reactive state (spec §8.6); runner.ts creates and feeds them.

import { ApiError, isAbort } from '../api';
import type { Conflict, Entry, Outcome } from '../types.gen';
import type { ActionHandle, ActionState, ApiFailure, ConnState, Spread } from '../types';
import { SEEN_TIMEOUT_MS } from './timing';

/** What the runner reads from and writes to the store. */
export interface RunnerHost {
  conn: ConnState;
}

/** How one action runs and how the runner knows it landed. */
export type ActionSpec<T> = {
  /** Sends the request; `signal` aborts it (only `latest` actions use it). */
  readonly run: (signal: AbortSignal) => Promise<T>;
  /** It signs a ledger op: the page shows `Signing…`, then `Saved on this device`. */
  readonly ledger?: boolean;
  /** It needs the OS to confirm first (§13 tier 3). */
  readonly tier3?: boolean;
  /** The journal entry that shows the change took effect, given the 2xx body. */
  readonly match?: (entry: Entry, result: T) => boolean;
  /** Loads the latest state after a 409 or 404, so the screen shows it (§12.5). */
  readonly refresh?: () => Promise<unknown>;
  /** A read: allowed while stale, never `slow`. */
  readonly read?: boolean;
  /** Only the newest call with this key counts; older ones abort and settle as `idle`. */
  readonly latest?: string;
};

/** What a handle that waits for its seen exposes to the runner. */
export interface Tracked {
  readonly op: string | null;
  observe(entry: Entry): void;
  markSeen(spread: Spread | null): void;
}

/** The runner as a handle sees it. */
export interface HandleOwner {
  readonly host: RunnerHost;
  track(handle: Tracked): void;
  untrack(handle: Tracked): void;
  takeAck(op: string): Spread | null;
}

// Entries that arrive before the 2xx answer are kept to match against it; a burst longer
// than this cannot belong to one action's window.
const EARLY_MAX = 64;

function outcomeOf(value: unknown): Outcome | null {
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  if (v['state'] !== 'accepted' && v['state'] !== 'awaiting_os_auth') return null;
  return { state: v['state'], op: typeof v['op'] === 'string' ? v['op'] : null };
}

export class Handle<T> implements ActionHandle<T>, Tracked {
  state = $state<ActionState>('pending');
  error = $state.raw<ApiFailure | null>(null);
  conflict = $state.raw<Conflict | null>(null);
  seen = $state.raw<Spread | null>(null);
  slow = $state(false);
  result = $state.raw<T | null>(null);
  readonly ledger: boolean;
  readonly done: Promise<ActionHandle<T>>;
  /** The ledger op id from the Outcome, once accepted. */
  op: string | null = null;

  private answered = false;
  private readonly signal: AbortSignal;
  private early: Entry[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly spec: ActionSpec<T>;
  private readonly runner: HandleOwner;

  constructor(spec: ActionSpec<T>, runner: HandleOwner, controller: AbortController | null, blocked: ApiError | null) {
    this.spec = spec;
    this.runner = runner;
    this.ledger = spec.ledger ?? false;
    if (spec.tier3) this.state = 'awaiting_os_auth';
    const signal = controller?.signal ?? new AbortController().signal;
    this.signal = signal;
    const run = blocked
      ? Promise.reject(blocked)
      : Promise.resolve().then(() => {
          signal.throwIfAborted();
          return spec.run(signal);
        });
    this.done = run.then(
      (value) => this.accept(value),
      (cause: unknown) => this.fail(cause),
    );
  }

  /** Called for every journal entry while this handle is tracked. */
  observe(entry: Entry): void {
    if (!this.spec.match) return;
    if (!this.answered) {
      if (this.early.length < EARLY_MAX) this.early.push(entry);
      return;
    }
    const result = this.result as T;
    if ((this.state === 'accepted' || this.state === 'awaiting_os_auth') && this.spec.match(entry, result)) {
      this.markSeen(null);
    }
  }

  /** Propagation from an ack: `by` of `of` devices have the op. */
  markSeen(spread: Spread | null): void {
    if (this.state === 'failed' || this.state === 'conflict' || this.state === 'idle') return;
    if (spread) this.seen = spread;
    this.state = 'seen';
    this.slow = false;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    // Keep listening while acks can still raise the count: an op the server named and
    // that not every device has heard yet.
    const complete = spread ? spread.by >= spread.of : this.seen === null && this.op === null;
    if (complete) this.runner.untrack(this);
    else this.runner.track(this);
  }

  private accept(value: T): ActionHandle<T> {
    // A newer call with the same `latest` key replaced this one while its answer was on the way.
    if (this.signal.aborted) return this.fail(this.signal.reason);
    this.answered = true;
    this.result = value;
    if (this.spec.read) {
      this.state = 'accepted';
      return this;
    }
    const outcome = outcomeOf(value);
    this.op = outcome?.op ?? null;
    this.state = outcome?.state === 'awaiting_os_auth' ? 'awaiting_os_auth' : 'accepted';
    const match = this.spec.match;
    const early = this.early;
    this.early = [];
    if (match && early.some((entry) => match(entry, value))) this.markSeen(null);
    const ack = this.op ? this.runner.takeAck(this.op) : null;
    if (ack) this.markSeen(ack);
    // markSeen may have moved the state on.
    if ((this.state as ActionState) === 'seen') return this;
    // Only wait for a seen that can come: a journal entry this action knows, or acks for
    // the op the server named.
    if (match || this.op) {
      this.runner.track(this);
      if (this.ledger) {
        this.timer = setTimeout(() => {
          this.timer = null;
          if (this.state === 'accepted') this.slow = true;
        }, SEEN_TIMEOUT_MS);
      }
    } else {
      this.runner.untrack(this);
    }
    return this;
  }

  private fail(cause: unknown): ActionHandle<T> {
    this.answered = true;
    this.early = [];
    this.runner.untrack(this);
    if (isAbort(cause)) {
      this.state = 'idle';
      return this;
    }
    const failure = ApiError.from(cause);
    if (failure.code === 'unauthorized') this.runner.host.conn = 'unauth';
    if (failure.code === 'conflict' && failure.conflict) {
      this.conflict = failure.conflict;
      this.state = 'conflict';
    } else {
      this.error = failure.toFailure();
      this.state = 'failed';
    }
    if ((failure.code === 'conflict' || failure.code === 'not_found') && this.spec.refresh) {
      void this.spec.refresh().catch(() => undefined);
    }
    return this;
  }
}
