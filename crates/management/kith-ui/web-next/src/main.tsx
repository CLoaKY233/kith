import { StrictMode, type ComponentType, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { startSession } from './lib/api';
import './fonts/fonts.css';
import './styles.css';

const host = document.getElementById('app')!;
const root = createRoot(host);
const render = (node: ReactNode) => root.render(<StrictMode>{node}</StrictMode>);

// The Console's pages before a session: the one-time setup link, an invited person's
// registration link, passkey sign-in and recovery. None shows the network.
const outside = /^\/(?:(setup|register)\/([^/]+)|(login)|(recover))\/?$/.exec(location.pathname);

if (outside) {
  const page: Promise<{ default: ComponentType<{ token: string }> }> =
    outside[1] === 'setup'
      ? import('./screens/auth/Setup')
      : outside[1] === 'register'
        ? import('./screens/auth/Register')
        : outside[4]
          ? import('./screens/auth/Recover')
          : import('./screens/auth/Login');
  const token = (() => {
    try {
      return decodeURIComponent(outside[2] ?? '');
    } catch {
      return '';
    }
  })();
  page.then(({ default: Page }) => render(<Page token={token} />)).catch(() => {
    host.textContent = 'This page could not load. Check your connection and reload.';
    host.style.cssText = 'padding:32px;font:14px system-ui,sans-serif';
  });
} else {
  // A sign-in link opened in a tab already on this page only changes the hash.
  addEventListener('hashchange', () => {
    if (/[#&]t=/.test(location.hash)) void startSession().then(() => location.reload());
  });
  void startSession().then((reason) => render(<App signedOutReason={reason} />));
}
