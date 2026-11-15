import { Drawer, DrawerBackdrop, DrawerContent, DrawerTitle, DrawerTrigger } from '@ui-sdk/react/drawer';
import { Menu } from 'lucide-react';
import { AccountMenu, AccountPanel } from './components/Account';
import { lazy, memo, Suspense, useEffect, useState, type ReactNode } from 'react';
import { Palette } from './components/Palette';
import { LiveProvider, useLive, useNetwork } from './lib/data';
import { Link, Router, useRoute } from './lib/router';
import { NAV, SETTINGS, isActive } from './nav';
import { Overview } from './screens/Overview';
import { SignedOut, type SignedOutReason } from './screens/auth/SignedOut';
import { Skeleton, Toaster } from './ui';
import { Welcome } from './screens/welcome/Welcome';

// Overview loads with the app; every other place loads when first opened.
const Access = lazy(() => import('./screens/Access').then((m) => ({ default: m.Access })));
const ActivityScreen = lazy(() => import('./screens/Activity').then((m) => ({ default: m.ActivityScreen })));
const DeviceDetail = lazy(() => import('./screens/DeviceDetail').then((m) => ({ default: m.DeviceDetail })));
const Devices = lazy(() => import('./screens/Devices').then((m) => ({ default: m.Devices })));
const Exits = lazy(() => import('./screens/Exits').then((m) => ({ default: m.Exits })));
const MapScreen = lazy(() => import('./screens/Map').then((m) => ({ default: m.MapScreen })));
const Names = lazy(() => import('./screens/Names').then((m) => ({ default: m.Names })));
const People = lazy(() => import('./screens/People').then((m) => ({ default: m.People })));
const SettingsScreen = lazy(() => import('./screens/Settings').then((m) => ({ default: m.SettingsScreen })));
const Stats = lazy(() => import('./screens/Stats').then((m) => ({ default: m.Stats })));

function SettingsBody({ slug }: { slug: string }) {
  switch (slug) {
    case 'exits': return <Exits />;
    case 'names': return <Names />;
    case 'access': return <Access />;
    case 'people': return <People />;
    default: return <SettingsScreen />;
  }
}

function SettingsArea({ slug }: { slug: string }) {
  return (
    <div className="grid md:min-h-[calc(100dvh-3.5rem)] md:grid-cols-[220px_minmax(0,1fr)]">
      <aside className="border-b border-line md:border-r md:border-b-0">
        <nav aria-label="Settings" className="flex gap-1 overflow-x-auto px-1.5 py-3 md:sticky md:top-0 md:flex-col md:overflow-visible md:p-4 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {SETTINGS.map((s) => {
            const on = s.slug === slug;
            return (
              <Link
                key={s.slug}
                to={`/settings/${s.slug}`}
                aria-current={on ? 'page' : undefined}
                className={`flex h-8 shrink-0 items-center whitespace-nowrap px-2.5 text-[13px] md:px-4 md:text-[14px] transition-colors duration-150 ${on ? 'bg-ink-2 text-fg' : 'text-fg-2 hover:text-fg'}`}
              >
                {s.label}
              </Link>
            );
          })}
        </nav>
      </aside>
      <div className="min-w-0">
        <SettingsBody slug={slug} />
      </div>
    </div>
  );
}

function deviceName(path: string): string | null {
  try {
    return decodeURIComponent(path.slice('/devices/'.length)) || null;
  } catch {
    return null;
  }
}

/** The settings section a path points at (unknown sections fall back to the first), or null outside settings. */
function settingsSlug(path: string): string | null {
  if (path !== '/settings' && !path.startsWith('/settings/')) return null;
  const slug = path.slice('/settings/'.length);
  return SETTINGS.some((s) => s.slug === slug) ? slug : SETTINGS[0].slug;
}

function pageTitle(path: string): string {
  let part: string | undefined;
  if (path.startsWith('/devices/')) part = deviceName(path) ?? 'Devices';
  else {
    const slug = settingsSlug(path);
    if (slug) part = `${SETTINGS.find((s) => s.slug === slug)?.label} settings`;
    else part = NAV.find((item) => item.path !== '/' && isActive(item, path))?.label;
  }
  return part ? `${part} · kith` : 'kith';
}

/** Shows the empty state when this device isn't in a network; otherwise the page. Re-renders on the poll without re-rendering the page. */
function Gate({ children }: { children: ReactNode }) {
  const { boot } = useLive();
  if (boot && boot.status.networks.length === 0) {
    return <Welcome />;
  }
  return children;
}

function Screen() {
  const { path, query } = useRoute();
  if (path.startsWith('/devices/')) {
    const name = deviceName(path);
    return name ? <DeviceDetail name={name} /> : <Devices />;
  }
  if (path === '/devices') return query.get('view') === 'map' ? <MapScreen /> : <Devices />;
  const slug = settingsSlug(path);
  if (slug) return <SettingsArea slug={slug} />;
  switch (path) {
    case '/stats': return <Stats />;
    case '/activity': return <ActivityScreen />;
    default: return <Overview />;
  }
}

function NavLinks({ onPick, stacked }: { onPick?: () => void; stacked?: boolean }) {
  const { path } = useRoute();
  return (
    <>
      {NAV.map((item) => {
        const on = isActive(item, path);
        return (
          <Link
            key={item.path}
            to={item.path === '/settings' ? '/settings/network' : item.path}
            onClick={onPick}
            aria-current={on ? 'page' : undefined}
            className={`flex items-center text-[14px] transition-colors duration-150 ${stacked ? `h-10 w-full border-l-2 pl-3 ${on ? 'border-fg' : 'border-transparent'}` : 'h-8 px-2'} ${on ? 'text-fg' : 'text-fg-2 hover:text-fg'}`}
          >
            {item.label}
          </Link>
        );
      })}
    </>
  );
}

function Status() {
  const { network } = useNetwork();
  const { error } = useLive();
  const text = error
    ? 'Offline'
    : !network
      ? 'Connecting'
      : network.peers_total === 0
        ? 'No devices'
        : `${network.peers_online} of ${network.peers_total} online`;
  return (
    <span
      className={`${error ? "flex" : "hidden sm:flex"} items-center gap-2 whitespace-nowrap font-mono text-[12px] text-fg-2`}
      title={error ? "Can't reach kith. Retrying." : text}
      role="status"
    >
      <span className={error ? 'dot-problem' : network ? 'dot-on' : 'dot-off'} aria-hidden="true" />
      <span className="sr-only sm:not-sr-only">{text}</span>
    </span>
  );
}

function TopBar() {
  const { network, boot } = useNetwork();
  const signedIn = Boolean(boot?.status.me);
  const [menu, setMenu] = useState(false);
  useEffect(() => {
    const wide = matchMedia('(min-width: 1024px)');
    const close = () => wide.matches && setMenu(false);
    wide.addEventListener('change', close);
    return () => wide.removeEventListener('change', close);
  }, []);
  return (
    <header className="sticky top-0 z-30 border-b border-line bg-ink-0/95">
      <div className="mx-auto flex h-14 max-w-[1280px] items-center gap-4 px-4 md:px-8 lg:gap-8">
        <Link to="/" className="flex h-8 min-w-0 items-center gap-2.5 whitespace-nowrap" aria-label="kith, overview">
          <svg width="16" height="14" viewBox="0 0 18 16" aria-hidden="true"><path d="M9 0L18 16H0Z" fill="currentColor" /></svg>
          <span className="flex min-w-0 items-baseline gap-2.5">
            <span className="text-[15px] leading-none font-medium tracking-tight">kith</span>
            {network && <span title={network.name} className="min-w-0 max-w-[16ch] truncate font-mono text-[12px] leading-none text-fg-3 sm:max-w-[28ch]">/ {network.name}</span>}
          </span>
        </Link>
        <nav aria-label="Main" className="hidden items-center gap-2 lg:-ml-2 lg:flex">
          <NavLinks />
        </nav>
        <div className="ml-auto flex shrink-0 items-center gap-3 sm:gap-4">
          <Status />
          <Palette className={signedIn ? '' : 'lg:-mr-3'} />
          <AccountMenu />
          <Drawer open={menu} onOpenChange={setMenu} side="left" preventEdgeSwipe>
            <DrawerTrigger className="grid !size-8 place-items-center !border-0 !bg-transparent !p-0 -mr-[7px] text-fg-2 transition-colors duration-150 hover:text-fg lg:hidden" aria-label="Open menu">
              <Menu size={18} />
            </DrawerTrigger>
            <DrawerBackdrop />
            <DrawerContent className="w-72">
              <DrawerTitle className="-mt-2 flex h-8 items-center gap-2.5 text-[15px] font-medium tracking-tight">
                <svg width="16" height="14" viewBox="0 0 18 16" aria-hidden="true"><path d="M9 0L18 16H0Z" fill="currentColor" /></svg>
                kith
              </DrawerTitle>
              <nav aria-label="Main" className="mt-6 flex flex-col">
                <NavLinks stacked onPick={() => setMenu(false)} />
              </nav>
              <AccountPanel />
            </DrawerContent>
          </Drawer>
        </div>
      </div>
    </header>
  );
}

function Loading() {
  return (
    <div className="p-4 md:p-8" role="status" aria-label="Loading">
      <Skeleton height={40} width={240} />
      <Skeleton className="mt-4" height={16} width={360} />
    </div>
  );
}

/** Everything around the page. Memoised so the 2 s poll doesn't re-render it; only the route does. */
const Chrome = memo(function Chrome() {
  const { path, query } = useRoute();
  useEffect(() => {
    document.title = pageTitle(path);
  }, [path]);
  // Re-key per place (not per settings tab or device) so the page fades in once on arrival.
  const place = path.startsWith('/devices/') ? path : path.startsWith('/settings') ? '/settings' : `${path}?${query.get('view') ?? ''}`;
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <main className="min-h-0 flex-1 overflow-y-auto">
        <div key={place} className="page-enter mx-auto min-h-full max-w-[1280px]">
          <Suspense fallback={<Loading />}>
            <Gate>
              <Screen />
            </Gate>
          </Suspense>
        </div>
      </main>
    </div>
  );
});

function Shell({ signedOutReason }: { signedOutReason: SignedOutReason }) {
  const { signedOut } = useLive();
  return signedOut ? <SignedOut reason={signedOutReason} /> : <Chrome />;
}

export function App({ signedOutReason }: { signedOutReason: SignedOutReason }) {
  return (
    <LiveProvider>
      <Router>
        <Shell signedOutReason={signedOutReason} />
        <Toaster />
      </Router>
    </LiveProvider>
  );
}
