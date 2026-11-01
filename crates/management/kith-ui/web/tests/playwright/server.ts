// Starts `kith-ui --fixture fixtures/office.json` on a free port and reads the one-time
// link it prints. Never the daemon: the fixture replays a recorded network.

import { spawn, type ChildProcess } from 'node:child_process';
import { fileURLToPath } from 'node:url';

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
  const child = spawn(binary, ['--fixture', fixturePath, '--no-open', '--port', '0'], {
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
      const match = LINK.exec(output);
      if (!match?.[1]) return;
      clearTimeout(timer);
      const link = match[0].slice('kith ui is at '.length);
      resolve({ origin: match[1], link, stop: stopper(child) });
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
