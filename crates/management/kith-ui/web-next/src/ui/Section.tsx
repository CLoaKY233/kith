import { useId, type ReactNode } from "react";

type Props = { title: ReactNode; description?: ReactNode; actions?: ReactNode; children?: ReactNode };

export function Section({ title, description, actions, children }: Props) {
  const id = useId();
  return (
    <section aria-labelledby={id}>
      <header className="mb-4 flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <div className="min-w-0">
          <h2 id={id} className="text-[20px] font-medium tracking-[-0.02em] text-fg [overflow-wrap:anywhere]">{title}</h2>
          {description && <p className="mt-1 max-w-[60ch] text-[13px] leading-5 text-fg-2 [overflow-wrap:anywhere]">{description}</p>}
        </div>
        {actions && <div className="flex max-w-full flex-wrap items-center gap-2 md:justify-end">{actions}</div>}
      </header>
      {children}
    </section>
  );
}
