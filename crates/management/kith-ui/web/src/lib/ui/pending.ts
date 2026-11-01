// When a pending Button shows its spinner (spec §5 Button, §8.6): never for actions that
// settle within SPINNER_DELAY_MS, and once shown, for at least SPINNER_MIN_MS, so a fast
// answer never flashes a spinner and a slow one never flickers.

import { SPINNER_DELAY_MS, SPINNER_MIN_MS } from '../live/timing';

export interface PendingGate {
  /** Reports whether the action is pending now. */
  set(pending: boolean): void;
  /** Cancels any timer; `onChange` is not called again. */
  dispose(): void;
}

/** Calls `onChange(true)` when the spinner should appear and `onChange(false)` when it should go. */
export function pendingGate(onChange: (shown: boolean) => void): PendingGate {
  let shown = false;
  let shownAt = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const clear = (): void => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };

  return {
    set(pending) {
      clear();
      if (pending) {
        if (shown) return;
        timer = setTimeout(() => {
          timer = null;
          shown = true;
          shownAt = Date.now();
          onChange(true);
        }, SPINNER_DELAY_MS);
        return;
      }
      if (!shown) return;
      const left = SPINNER_MIN_MS - (Date.now() - shownAt);
      const hide = (): void => {
        timer = null;
        shown = false;
        onChange(false);
      };
      if (left <= 0) hide();
      else timer = setTimeout(hide, left);
    },
    dispose: clear,
  };
}
