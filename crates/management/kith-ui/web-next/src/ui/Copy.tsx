import { useEffect, useRef, useState } from "react";

type State = "idle" | "copied" | "failed";

async function write(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the legacy path (insecure origins, denied permission)
  }
  const el = document.createElement("textarea");
  el.value = text;
  el.setAttribute("readonly", "");
  el.style.cssText = "position:fixed;top:0;left:0;opacity:0";
  document.body.appendChild(el);
  el.select();
  try {
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    el.remove();
  }
}

export function Copy({ text, label }: { text: string; label?: string }) {
  const [state, setState] = useState<State>("idle");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const run = () =>
    void write(text).then((ok) => {
      setState(ok ? "copied" : "failed");
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setState("idle"), 1200);
    });

  return (
    <button
      type="button"
      onClick={run}
      title={state === "copied" ? "Copied" : state === "failed" ? "Couldn't copy" : "Copy"}
      aria-label={label ?? `Copy ${text}`}
      className="inline-flex max-w-full min-w-0 items-center text-left font-mono text-[13px] text-fg transition-colors duration-150 hover:text-fg-2 outline-offset-2 leading-5 focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg"
    >
      <span className="truncate">{label ?? text}</span>
      <span aria-live="polite" className={`shrink-0 font-mono text-[11px] uppercase tracking-[0.08em] ${state === "failed" ? "text-hot" : "text-fg-3"} ${state === "idle" ? "" : "ml-2"}`}>
        {state === "copied" ? "copied" : state === "failed" ? "failed" : ""}
      </span>
    </button>
  );
}
