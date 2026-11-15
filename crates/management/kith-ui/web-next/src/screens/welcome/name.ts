import { useCallback, useState } from 'react';
import type { NameCheck, NameProblem } from '@kith/types';
import { api } from '../../lib/api';
import { useLive } from '../../lib/data';

const KEY = 'kith.welcome-name';

function stored(): string | null {
  try {
    return sessionStorage.getItem(KEY);
  } catch {
    return null;
  }
}

/** The name this device starts or joins its first network with: the host name until renamed for this tab. */
export function useDeviceName() {
  const { boot } = useLive();
  const [chosen, setChosen] = useState<string | null>(stored);
  const set = useCallback((value: string) => {
    setChosen(value);
    try {
      sessionStorage.setItem(KEY, value);
    } catch {
      // Storage is off; the name lasts until the page reloads.
    }
  }, []);
  return { name: chosen ?? boot?.status.host.name ?? '', os: boot?.status.host.os ?? '', set };
}

/** Lowercases and turns spaces into hyphens, the way kith stores names. */
export const normalise = (raw: string) => raw.toLowerCase().replace(/ /g, '-');

const PROBLEM: Record<Exclude<NameProblem, 'taken'>, string> = {
  empty: 'Give it a name.',
  too_long: 'Names can be up to 63 characters.',
  invalid_character: 'Names can use a–z, 0–9 and hyphens.',
  hyphen_at_edge: "Names can't start or end with a hyphen.",
};

/** Checks a name with kith; answers the stored name, or the sentence that says what is wrong. */
export async function checkName(name: string, taken: string): Promise<{ name: string; problem: string | null; suggestion: string | null }> {
  const r = await api.get<NameCheck>(`/api/validate/name?net=&name=${encodeURIComponent(name)}`);
  const problem = r.problem === null ? null : r.problem === 'taken' ? taken : PROBLEM[r.problem];
  return { name: r.name, problem, suggestion: r.suggestion };
}
