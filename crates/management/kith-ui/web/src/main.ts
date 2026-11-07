// Entry: trade the one-time link token for the session cookie (spec §2.1), drop the
// fragment, then hydrate the store and mount the app.

import '../tailwind.css';
import { mount, type Component } from 'svelte';
import App from './App.svelte';
import { api } from './lib/api';
import type { ApiFailure, GateReason } from './lib/types';
import { parseBoot } from './lib/live/boot';
import { start } from './lib/live/store.svelte';
import { preload } from './screens.svelte';

/** Trades the token; answers why it was refused when the link was used or expired. */
async function signIn(): Promise<GateReason | null> {
  const match = /(?:^#|&)t=([0-9a-f]+)/.exec(location.hash);
  if (!match?.[1]) return null;
  history.replaceState(null, '', `${location.pathname}${location.search}`);
  try {
    await api.post('/api/session', { token: match[1] });
    return null;
  } catch (cause) {
    const code = (cause as Partial<ApiFailure> | null)?.code;
    // The Gate shows it once the store's first read finds no session.
    return code === 'session_used' || code === 'session_expired' ? code : null;
  }
}

async function boot(): Promise<void> {
  const target = document.getElementById('app');
  if (!target) return;
  // The component kit (spec §16 B), only under `npm run dev`; production builds drop it.
  if (import.meta.env.DEV && location.pathname === '/__kit') {
    const { default: Kit } = await import('./kit/Kit.svelte');
    target.replaceChildren();
    mount(Kit, { target });
    return;
  }
  // The Console's pages before a session: the one-time setup link, an invited person's
  // registration link, and passkey sign-in. None shows the network.
  const outside = /^\/(?:(setup|register)\/([^/]+)|login)\/?$/.exec(location.pathname);
  if (outside) {
    const page =
      outside[1] === 'setup'
        ? import('./routes/Setup.svelte')
        : outside[1] === 'register'
          ? import('./routes/Register.svelte')
          : import('./routes/Login.svelte');
    const { default: Page } = (await page) as { default: Component<{ token: string }> };
    target.replaceChildren();
    mount(Page, { target, props: { token: outside[2] ?? '' } });
    return;
  }
  const snapshot = parseBoot(document.getElementById('boot')?.textContent);
  const [signedOut] = await Promise.all([signIn(), preload(location.pathname, snapshot)]);
  target.replaceChildren();
  mount(App, { target });
  await start(signedOut);
}

void boot();
