import type { ReactNode } from 'react';

/** The calm, centred frame for pages shown before or without a session. */
export function AuthPage({ title, description, children }: { title: ReactNode; description?: ReactNode; children?: ReactNode }) {
  return (
    <main className="page-enter mx-auto flex min-h-full w-full max-w-[432px] flex-col justify-center gap-8 px-4 py-12 sm:py-16">
      <a href="/" className="flex w-fit items-center gap-2.5 text-fg no-underline outline-offset-4 focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg">
        <svg width="16" height="14" viewBox="0 0 18 16" aria-hidden="true"><path d="M9 0L18 16H0Z" fill="currentColor" /></svg>
        <span className="text-[15px] font-medium tracking-tight">kith</span>
      </a>
      <header>
        <h1 className="[overflow-wrap:anywhere] [text-wrap:balance] text-[28px] font-[450] leading-[1.1] tracking-[-0.03em] text-fg">{title}</h1>
        {description && <p className="mt-2 max-w-none text-[14px] leading-6 text-fg-2 [text-wrap:pretty]">{description}</p>}
      </header>
      <div className="flex flex-col gap-6">{children}</div>
    </main>
  );
}

export function Problem({ children }: { children: ReactNode }) {
  return <p role="alert" className="max-w-none text-[13px] leading-5 text-hot [overflow-wrap:anywhere]">{children}</p>;
}

export function Note({ children }: { children: ReactNode }) {
  return <p className="max-w-none text-[14px] leading-6 text-fg-2 [overflow-wrap:anywhere] [text-wrap:pretty]">{children}</p>;
}

/** A quiet text link under the main action. */
export function AuthLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} className="w-fit text-[13px] text-fg-2 outline-offset-4 transition-colors hover:text-fg focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg">
      {children}
    </a>
  );
}
