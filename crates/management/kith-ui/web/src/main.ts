// Entry: trade the one-time link token for the session cookie (spec §2.1), drop the
// fragment, then hydrate the store and mount the app.

import '../tailwind.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { api } from './lib/api';
import { start } from './lib/live/store.svelte';

async function signIn(): Promise<void> {
  const match = /(?:^#|&)t=([0-9a-f]+)/.exec(location.hash);
  if (!match?.[1]) return;
  history.replaceState(null, '', `${location.pathname}${location.search}`);
  try {
    await api.post('/api/session', { token: match[1] });
  } catch {
    // The store's first read reports why (used, expired); the Gate shows it.
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
  await signIn();
  target.replaceChildren();
  mount(App, { target });
  await start();
}

void boot();
