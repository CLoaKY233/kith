import type { ReactNode } from "react";

export function Empty({ icon, children, action }: { icon?: ReactNode; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-4 py-8 text-center">
      {icon && <span className="text-fg-3 [&>svg]:size-5" aria-hidden="true">{icon}</span>}
      <p className="max-w-[40ch] text-[14px] leading-5 [overflow-wrap:anywhere] text-fg-2 [text-wrap:balance]">{children}</p>
      {action}
    </div>
  );
}
