// Starts `kith-ui --fixture fixtures/office.json` on a free port and reads the one-time
// link it prints, or, for the Console, a fixture device in no network with the setup link
// `--fixture-console` prints. Never the daemon: the fixture replays a recorded network.

import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Fixture } from '../../src/lib/types.gen';

const web = fileURLToPath(new URL('../../', import.meta.url));

/** The binary under test: `KITH_UI_BIN`, or the workspace's debug build. */
export const binary = process.env['KITH_UI_BIN'] ?? `${web}../../../../target/debug/kith-ui`;
export const fixturePath = `${web}fixtures/office.json`;
export const distDir = `${web}dist`;

const LINK = /kith ui is at (http:\/\/127\.0\.0\.1:(\d+))\/#t=[0-9a-f]{64}/;
const START_TIMEOUT_MS = 10_000;

export interface Server {
  /** `http://127.0.0.1:<port>`. */
  origin: string;
  /** The one-time link, `<origin>/#t=<token>`. */
  link: string;
  stop(): Promise<void>;
}

function stopper(child: ChildProcess): () => Promise<void> {
  return () =>
    new Promise((resolve) => {
      if (child.exitCode !== null || child.signalCode !== null) return resolve();
      child.once('exit', () => resolve());
      child.kill('SIGTERM');
    });
}

export function startServer(): Promise<Server> {
  return spawnServer(['--fixture', fixturePath], (output, stop) => {
    const match = LINK.exec(output);
    if (!match?.[1]) return null;
    return { origin: match[1], link: match[0].slice('kith ui is at '.length), stop };
  });
}

const SETUP = /^\s+(http:\/\/localhost:\d+)\/setup\/[0-9a-f]{64}$/m;

/** A server for the Console's flows: a device in no network yet, and its setup link. */
export interface ConsoleServer {
  /** `http://localhost:<port>`: passkeys work locally only on `localhost`. */
  origin: string;
  /** `<origin>/setup/<token>`, good once. */
  setup: string;
  stop(): Promise<void>;
}

/**
 * The office recording with every network taken out: a device that is in no network yet,
 * as `install.sh --console` finds it, so setup names and creates the network.
 */
function freshFixture(): string {
  const office = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
  const fresh: Fixture = {
    ...office,
    status: { ...office.status, networks: [], memberships: [], exit_node: null, imported: [], warnings: [] },
    peers: {},
    records: {},
    pending: { joins: [], blocked: [] },
    journal: [],
    traffic: [],
    links: {},
    exits: {},
    invites: {},
    policies: {},
    relays: {},
  };
  const path = join(mkdtempSync(join(tmpdir(), 'kith-ui-console-')), 'fresh.json');
  writeFileSync(path, JSON.stringify(fresh));
  return path;
}

/** `kith-ui --fixture <fresh> --fixture-console`: setup, passkeys and people in memory. */
export function startConsole(): Promise<ConsoleServer> {
  return spawnServer(['--fixture', freshFixture(), '--fixture-console'], (output, stop) => {
    const match = SETUP.exec(output);
    if (!match?.[1]) return null;
    return { origin: match[1], setup: match[0].trim(), stop };
  });
}

function spawnServer<T>(args: readonly string[], ready: (output: string, stop: () => Promise<void>) => T | null): Promise<T> {
  const child = spawn(binary, [...args, '--no-open', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`kith-ui printed no link within ${START_TIMEOUT_MS} ms:\n${output}`));
    }, START_TIMEOUT_MS);
    const read = (chunk: Buffer): void => {
      output += chunk.toString();
      const server = ready(output, stopper(child));
      if (server === null) return;
      clearTimeout(timer);
      resolve(server);
    };
    child.stdout?.on('data', read);
    child.stderr?.on('data', (chunk: Buffer) => (output += chunk.toString()));
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(new Error(`cannot run ${binary}: ${error.message}. Run cargo build -p kith-ui.`));
    });
    child.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`kith-ui exited with ${String(code)} before printing a link:\n${output}`));
    });
  });
}
