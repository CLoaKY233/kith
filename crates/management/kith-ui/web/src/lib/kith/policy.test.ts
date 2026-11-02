import { describe, expect, it } from 'vitest';
import type { PolicyRule } from '../types';
import { errorPosition, offsetOf, parsePorts, policyToml, readRules, ruleSentence } from './policy';

const NAS = 'ab'.repeat(32);
const names = (node: string): string | null => (node === NAS ? 'nas' : null);

const sshToNas: PolicyRule = {
  sources: [{ kind: 'tag', tag: 'phones' }],
  destinations: [{ selector: { kind: 'device', node: NAS }, protocol: 'tcp', ports: [{ start: 22, end: 22 }] }],
};

describe('policy sentences', () => {
  it('reads a rule as a sentence', () => {
    expect(ruleSentence(sshToNas, names)).toBe('Phones can reach nas on TCP port 22.');
    expect(
      ruleSentence(
        {
          sources: [{ kind: 'any' }],
          destinations: [
            { selector: { kind: 'any' }, protocol: 'icmp', ports: null },
            { selector: { kind: 'device', node: NAS }, protocol: 'any', ports: [{ start: 80, end: 90 }, { start: 443, end: 443 }] },
          ],
        },
        names,
      ),
    ).toBe('Every device can reach every device by ping and nas on ports 80–90 and 443.');
  });

  it('names a device that left by its short key', () => {
    const rule = { ...sshToNas, sources: [{ kind: 'device' as const, node: 'cd'.repeat(32) }] };
    expect(ruleSentence(rule, names)).toMatch(/^a device no longer here \(cdcdcdcd\) can reach/);
  });

  it('reads the daemon serde shape as well as its own', () => {
    const serde = {
      rules: [
        {
          sources: [{ Tag: 'phones' }],
          destinations: [
            { selector: { Device: Array(32).fill(0xab) }, protocol: 'Tcp', ports: { Only: [{ start: 22, end: 22 }] } },
          ],
        },
      ],
    };
    expect(readRules(serde)).toEqual([sshToNas]);
    expect(readRules([sshToNas])).toEqual([sshToNas]);
    expect(readRules([{ sources: ['Nope'], destinations: [] }])).toBeNull();
  });
});

describe('policy editing', () => {
  it('parses port lists', () => {
    expect(parsePorts('')).toBeNull();
    expect(parsePorts('22, 8000-8080')).toEqual([
      { start: 22, end: 22 },
      { start: 8000, end: 8080 },
    ]);
    expect(parsePorts('0')).toBe('invalid');
    expect(parsePorts('90-80')).toBe('invalid');
    expect(parsePorts('ssh')).toBe('invalid');
  });

  it('writes the TOML the daemon reads', () => {
    const toml = policyToml([sshToNas]);
    expect(toml).toContain('[[rules]]\nsources = [{ Tag = "phones" }]');
    expect(toml).toContain(`selector = { Device = [${Array(32).fill(171).join(', ')}] }`);
    expect(toml).toContain('protocol = "Tcp", ports = { Only = [{ start = 22, end = 22 }] }');
    expect(policyToml([])).toBe('rules = []\n');
  });

  it('finds where a parse error points', () => {
    const message = 'TOML parse error at line 2, column 11\n  |\n2 | sources = [\n  |           ^\ninvalid array';
    expect(errorPosition(message)).toEqual({ line: 2, column: 11 });
    expect(errorPosition('no position')).toBeNull();
    expect(offsetOf('ab\ncdef\n', 2, 3)).toBe(5);
    expect(offsetOf('ab', 9, 9)).toBe(2);
  });
});
