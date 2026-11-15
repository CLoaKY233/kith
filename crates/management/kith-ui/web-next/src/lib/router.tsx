import { createContext, useCallback, useContext, useEffect, useMemo, useState, type AnchorHTMLAttributes, type MouseEvent, type ReactNode } from 'react';

type Route = {
  path: string;
  query: URLSearchParams;
  go: (to: string, options?: { replace?: boolean }) => void;
};
const RouterContext = createContext<Route>({ path: '/', query: new URLSearchParams(), go: () => {} });

const here = () => ({ path: location.pathname.replace(/\/+$/, '') || '/', search: location.search });

const sync = (prev: ReturnType<typeof here>) => {
  const next = here();
  return next.path === prev.path && next.search === prev.search ? prev : next;
};

export function Router({ children }: { children: ReactNode }) {
  const [at, setAt] = useState(here);
  useEffect(() => {
    const onPop = () => setAt(sync);
    addEventListener('popstate', onPop);
    return () => removeEventListener('popstate', onPop);
  }, []);
  const go = useCallback((to: string, options?: { replace?: boolean }) => {
    if (to === location.pathname + location.search) return;
    const moved = new URL(to, location.href).pathname !== location.pathname;
    history[options?.replace ? 'replaceState' : 'pushState'](null, '', to);
    setAt(sync);
    if (moved && !options?.replace) document.querySelector('main')?.scrollTo({ top: 0 });
  }, []);
  const value = useMemo(() => ({ path: at.path, query: new URLSearchParams(at.search), go }), [at, go]);
  return <RouterContext.Provider value={value}>{children}</RouterContext.Provider>;
}

/** A real link: opens in a new tab with ctrl/cmd-click, navigates in place otherwise. */
export function Link({ to, onClick, className = '', ...rest }: { to: string } & Omit<AnchorHTMLAttributes<HTMLAnchorElement>, 'href'>) {
  const { go } = useRoute();
  const click = (e: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(e);
    if (e.defaultPrevented || e.button !== 0 || (rest.target && rest.target !== '_self') || rest.download !== undefined || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    // A mouse click shouldn't leave a focus ring behind; keyboard activation (detail 0) keeps focus.
    if (e.detail > 0) e.currentTarget.blur();
    go(to);
  };
  return <a {...rest} href={to} onClick={click} className={`no-underline ${className}`} />;
}

export const useRoute = () => useContext(RouterContext);
