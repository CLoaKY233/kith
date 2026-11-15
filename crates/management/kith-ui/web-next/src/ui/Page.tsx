import type { ReactNode } from "react";

type Props = { title: ReactNode; description?: ReactNode; actions?: ReactNode; children?: ReactNode };

export function Page({ title, description, actions, children }: Props) {
  return (
    <div className="mx-auto w-full max-w-[1280px] px-4 py-6 md:px-8 md:py-8">
      <header className="mb-6 grid grid-cols-1 gap-x-6 md:mb-8 md:grid-cols-[minmax(0,1fr)_auto]">
        <h1 className="min-w-0 self-center text-[28px] font-[450] leading-[1.1] tracking-[-0.035em] text-fg [overflow-wrap:anywhere] [text-wrap:balance] md:text-[40px]">{title}</h1>
        {actions && <div className="order-last mt-4 flex max-w-full flex-wrap items-center gap-2 md:order-none md:mt-0 md:justify-end md:self-center">{actions}</div>}
        {description && <p className="mt-2 max-w-[60ch] text-[14px] leading-5 text-fg-2 [overflow-wrap:anywhere] md:col-span-2">{description}</p>}
      </header>
      <div className="flex flex-col gap-10 md:gap-12">{children}</div>
    </div>
  );
}
