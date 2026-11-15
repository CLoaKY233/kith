// Fails when the built page, as kith-ui embeds it (brotli, quality 11), outgrows its budget.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { brotliCompressSync, constants } from 'node:zlib';

const BUDGET_KB = 380;
const dist = new URL('../../web/dist/', import.meta.url).pathname;
let total = 0;
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else total += brotliCompressSync(readFileSync(path), { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
  }
};
walk(dist);
const kb = Math.round(total / 1024);
console.log(`page: ${kb} KB brotli (budget ${BUDGET_KB} KB)`);
if (kb > BUDGET_KB) process.exit(1);
