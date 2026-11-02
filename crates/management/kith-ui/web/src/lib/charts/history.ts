// The boot snapshot carries only the last few minutes of traffic; the server keeps the
// hour (§9.1). The first chart on the page asks for it once and puts the older samples in
// front of what the store already has.

import { api } from '../api';
import { deviceKey } from '../live/keys';
import { Columns } from '../live/traffic';
import type { TrafficColumns } from '../types';
import type { TrafficSeries } from '../types.gen';

let loading: Promise<void> | null = null;

/** Puts `samples` older than the first one in `columns` in front of it, in place. */
export function prepend(
    columns: Columns,
    samples: readonly { at_ms: number; rx_bytes: number; tx_bytes: number }[],
): number {
    const first = columns.length > 0 ? (columns.at[0] ?? Infinity) : Infinity;
    const older = samples
        .filter((s) => s.at_ms < first)
        .sort((a, b) => a.at_ms - b.at_ms);
    const room = columns.capacity - columns.length;
    const add = older.slice(Math.max(0, older.length - room));
    if (add.length === 0) return 0;
    const n = add.length;
    columns.at.copyWithin(n, 0, columns.length);
    columns.rx.copyWithin(n, 0, columns.length);
    columns.tx.copyWithin(n, 0, columns.length);
    add.forEach((s, i) => {
        columns.at[i] = s.at_ms;
        columns.rx[i] = s.rx_bytes;
        columns.tx[i] = s.tx_bytes;
    });
    columns.length += n;
    return n;
}

/** Loads the hour once per page; resolves when the store's columns hold it. */
export function loadHistory(
    traffic: ReadonlyMap<string, TrafficColumns>,
): Promise<void> {
    loading ??= api.get<TrafficSeries[]>('/api/traffic').then(
        (all) => {
            for (const series of all) {
                const columns = traffic.get(
                    deviceKey(series.network, series.node),
                );
                if (columns instanceof Columns)
                    prepend(columns, series.samples);
            }
        },
        () => {
            loading = null;
        },
    );
    return loading;
}
