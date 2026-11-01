// Refuses to test a binary that embeds the fallback page or an older build of the page:
// every script and style the served page loads must be in today's `dist/`.

import { existsSync, readFileSync } from 'node:fs';
import { distDir, startServer } from './server';

const REBUILD = 'Run npm run build, then cargo build -p kith-ui.';

export default async function globalSetup(): Promise<void> {
  if (!existsSync(`${distDir}/index.html`)) throw new Error(`dist/ is missing. ${REBUILD}`);
  const server = await startServer();
  try {
    const html = await (await fetch(`${server.origin}/`)).text();
    const served = [...html.matchAll(/(?:src|href)="\/(assets\/[^"]+)"/g)].map((m) => m[1] ?? '');
    if (served.length === 0) throw new Error(`kith-ui embeds fallback.html. ${REBUILD}`);
    const built = readFileSync(`${distDir}/index.html`, 'utf8');
    const stale = served.filter((asset) => !built.includes(`/${asset}"`));
    if (stale.length > 0) {
      throw new Error(`kith-ui embeds an older page (${stale.join(', ')}). ${REBUILD}`);
    }
  } finally {
    await server.stop();
  }
}
