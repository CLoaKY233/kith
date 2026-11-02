// The network policy (M1.1 Access): rules as plain sentences, the structured editor's
// helpers, and the TOML the daemon reads. A policy is a set of allow rules only; anything
// no rule allows is blocked. The TOML is the daemon's own encoding of its Policy type, so
// a device is its key as 32 bytes and variants are `Any`, `{ Device = [...] }`,
// `{ Tag = "phones" }`.

import { copy } from '../copy';
import type {
  PolicyProtocol,
  PolicyRule,
  PolicySelector,
  PolicyTarget,
  PortRange,
} from '../types';

const PORT_MAX = 65535;

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function hex(bytes: readonly unknown[]): string | null {
  if (!bytes.every((b) => typeof b === 'number' && b >= 0 && b <= 255)) return null;
  return (bytes as number[]).map((b) => b.toString(16).padStart(2, '0')).join('');
}

function node(value: unknown): string | null {
  if (typeof value === 'string') return value;
  return Array.isArray(value) ? hex(value) : null;
}

/** A selector as the server sends it: this page's shape, or the daemon's serde shape. */
export function readSelector(value: unknown): PolicySelector | null {
  if (typeof value === 'string') return value.toLowerCase() === 'any' ? { kind: 'any' } : null;
  const v = record(value);
  if (!v) return null;
  if (v['kind'] === 'any') return { kind: 'any' };
  if (v['kind'] === 'device' && typeof v['node'] === 'string') return { kind: 'device', node: v['node'] };
  if (v['kind'] === 'tag' && typeof v['tag'] === 'string') return { kind: 'tag', tag: v['tag'] };
  const device = node(v['Device'] ?? v['device']);
  if (device !== null) return { kind: 'device', node: device };
  const tag = v['Tag'] ?? v['tag'];
  return typeof tag === 'string' ? { kind: 'tag', tag } : null;
}

function readProtocol(value: unknown): PolicyProtocol | null {
  if (typeof value !== 'string') return null;
  const p = value.toLowerCase();
  return p === 'any' || p === 'tcp' || p === 'udp' || p === 'icmp' ? p : null;
}

function readRange(value: unknown): PortRange | null {
  if (Array.isArray(value) && value.length === 2) {
    const [start, end] = value as unknown[];
    return typeof start === 'number' && typeof end === 'number' ? { start, end } : null;
  }
  const v = record(value);
  if (!v) return typeof value === 'number' ? { start: value, end: value } : null;
  const start = v['start'];
  const end = v['end'];
  return typeof start === 'number' && typeof end === 'number' ? { start, end } : null;
}

/** `null` for every port; `undefined` when the value is not a port set. */
function readPorts(value: unknown): readonly PortRange[] | null | undefined {
  if (value === null || value === undefined) return null;
  if (typeof value === 'string') return value.toLowerCase() === 'all' ? null : undefined;
  const v = record(value);
  const list = v ? (v['Only'] ?? v['only']) : value;
  if (!Array.isArray(list)) return undefined;
  const ranges = list.map(readRange);
  return ranges.every((r): r is PortRange => r !== null) ? ranges : undefined;
}

function readTarget(value: unknown): PolicyTarget | null {
  const v = record(value);
  if (!v) return null;
  const selector = readSelector(v['selector']);
  const protocol = readProtocol(v['protocol'] ?? 'any');
  const ports = readPorts(v['ports']);
  if (!selector || !protocol || ports === undefined) return null;
  return { selector, protocol, ports };
}

/** The rules the server listed, or null when they are not in a shape this page reads. */
export function readRules(value: unknown): PolicyRule[] | null {
  const list = Array.isArray(value) ? value : (record(value)?.['rules'] ?? null);
  if (!Array.isArray(list)) return null;
  const rules: PolicyRule[] = [];
  for (const item of list) {
    const v = record(item);
    const sources = Array.isArray(v?.['sources']) ? (v['sources'] as unknown[]).map(readSelector) : null;
    const destinations = Array.isArray(v?.['destinations'])
      ? (v['destinations'] as unknown[]).map(readTarget)
      : null;
    if (!sources || !destinations) return null;
    if (sources.some((s) => s === null) || destinations.some((d) => d === null)) return null;
    rules.push({
      sources: sources as PolicySelector[],
      destinations: destinations as PolicyTarget[],
    });
  }
  return rules;
}

/** How a sentence names a device: its name, or a short key when it is not in the network. */
export type NameOf = (node: string) => string | null;

export function selectorText(selector: PolicySelector, nameOf: NameOf): string {
  switch (selector.kind) {
    case 'any':
      return copy.access.everyDevice;
    case 'tag':
      return selector.tag;
    case 'device':
      return nameOf(selector.node) ?? copy.access.unknownDevice(selector.node.slice(0, 8));
  }
}

export function rangeText(range: PortRange): string {
  return range.start === range.end ? `${range.start}` : `${range.start}–${range.end}`;
}

export function targetText(target: PolicyTarget, nameOf: NameOf): string {
  const who =
    target.selector.kind === 'any' ? copy.access.everyDeviceAfter : selectorText(target.selector, nameOf);
  if (target.protocol === 'icmp') return copy.access.byPing(who);
  const proto = target.protocol === 'any' ? null : target.protocol.toUpperCase();
  if (target.ports === null) return proto === null ? who : copy.access.overProtocol(who, proto);
  const single = target.ports.length === 1 && target.ports[0]?.start === target.ports[0]?.end;
  return copy.access.onPorts(who, proto, single, copy.exits.list(target.ports.map(rangeText)));
}

/** `Phones can reach nas on port 22.` A tag that starts the sentence takes a capital. */
export function ruleSentence(rule: PolicyRule, nameOf: NameOf): string {
  let who = copy.exits.list(rule.sources.map((s) => selectorText(s, nameOf)));
  if (rule.sources[0]?.kind === 'tag') who = who.charAt(0).toUpperCase() + who.slice(1);
  const what = copy.exits.list(rule.destinations.map((d) => targetText(d, nameOf)));
  return copy.access.canReach(who, what);
}

/**
 * Ports as typed in the editor: `22`, `8000-8080`, `22, 80, 443`. Empty is every port
 * (null); anything else that is not a valid list is `invalid`.
 */
export function parsePorts(text: string): PortRange[] | null | 'invalid' {
  const trimmed = text.trim();
  if (trimmed === '') return null;
  const ranges: PortRange[] = [];
  for (const part of trimmed.split(/[\s,]+/).filter(Boolean)) {
    const match = /^(\d{1,5})(?:\s*[-–]\s*(\d{1,5}))?$/.exec(part);
    if (!match) return 'invalid';
    const start = Number(match[1]);
    const end = match[2] === undefined ? start : Number(match[2]);
    if (start < 1 || end > PORT_MAX || start > end) return 'invalid';
    ranges.push({ start, end });
  }
  return ranges;
}

function bytes(nodeHex: string): string {
  if (!/^[0-9a-f]{64}$/i.test(nodeHex)) return JSON.stringify(nodeHex);
  const out: number[] = [];
  for (let i = 0; i < 64; i += 2) out.push(parseInt(nodeHex.slice(i, i + 2), 16));
  return `[${out.join(', ')}]`;
}

function selectorToml(selector: PolicySelector): string {
  switch (selector.kind) {
    case 'any':
      return '"Any"';
    case 'tag':
      return `{ Tag = ${JSON.stringify(selector.tag)} }`;
    case 'device':
      return `{ Device = ${bytes(selector.node)} }`;
  }
}

const PROTOCOL_TOML: Readonly<Record<PolicyProtocol, string>> = {
  any: 'Any',
  tcp: 'Tcp',
  udp: 'Udp',
  icmp: 'Icmp',
};

function targetToml(target: PolicyTarget): string {
  const ports =
    target.ports === null
      ? '"All"'
      : `{ Only = [${target.ports.map((r) => `{ start = ${r.start}, end = ${r.end} }`).join(', ')}] }`;
  return `{ selector = ${selectorToml(target.selector)}, protocol = "${PROTOCOL_TOML[target.protocol]}", ports = ${ports} }`;
}

/** The policy as the daemon reads it. */
export function policyToml(rules: readonly PolicyRule[]): string {
  if (rules.length === 0) return 'rules = []\n';
  return rules
    .map(
      (rule) =>
        `[[rules]]\nsources = [${rule.sources.map(selectorToml).join(', ')}]\ndestinations = [\n${rule.destinations
          .map((d) => `  ${targetToml(d)},`)
          .join('\n')}\n]\n`,
    )
    .join('\n');
}

/** Where a TOML parse error points, from the server's 400 message. */
export function errorPosition(message: string): { line: number; column: number } | null {
  const match = /line\s+(\d+)\s*,?\s*col(?:umn)?\s+(\d+)/i.exec(message);
  if (!match) return null;
  return { line: Number(match[1]), column: Number(match[2]) };
}

/** The offset in `text` of a 1-based line and column, clamped to the text. */
export function offsetOf(text: string, line: number, column: number): number {
  const lines = text.split('\n');
  let offset = 0;
  for (let i = 0; i < Math.min(line - 1, lines.length - 1); i++) offset += (lines[i]?.length ?? 0) + 1;
  const here = lines[Math.min(line, lines.length) - 1] ?? '';
  return offset + Math.min(Math.max(column - 1, 0), here.length);
}

/** Every tag the rules mention, for the editor's pickers. */
export function tagsIn(rules: readonly PolicyRule[]): string[] {
  const tags = new Set<string>();
  for (const rule of rules) {
    for (const s of [...rule.sources, ...rule.destinations.map((d) => d.selector)]) {
      if (s.kind === 'tag') tags.add(s.tag);
    }
  }
  return [...tags].sort();
}
