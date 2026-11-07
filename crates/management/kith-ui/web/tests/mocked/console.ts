// A mocked Console for the page under `vite preview`: every /api call is answered here,
// in the shapes of docs/plans/kith-ui-api.md, from the office fixture plus a little account
// state; the event stream is a fake the test pushes entries into, and Chrome's virtual
// authenticator plays the passkey. The real server's flow is tests/playwright/console.spec.ts;
// these cover the failures that one cannot cause on demand.

import { test as base, expect, type Page, type Route } from '@playwright/test';
import { generateKeyPairSync, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { Boot, Entry, Fixture, Me, PeerInfo, PersonView } from '../../src/lib/types.gen';

const fixturePath = fileURLToPath(new URL('../../fixtures/office.json', import.meta.url));
export const office = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
const firstNetwork = office.status.networks[0];
if (!firstNetwork) throw new Error('the office fixture has no network');
export const network = firstNetwork;
const firstPeer: PeerInfo | undefined = office.peers[network.id]?.[0];
if (!firstPeer) throw new Error('the office fixture has no device');
/** Bo's device, from the recording. */
export const boDevice = firstPeer;
const SEQ = 1000;

export const ana: Me = { id: 'a-ana', name: 'Ana', role: 'owner' };
export const bo: Me = { id: 'a-bo', name: 'Bo', role: 'admin' };
export const cy: Me = { id: 'a-cy', name: 'Cy', role: 'member' };

/** `who` as `GET /api/people` lists them. */
export function view(who: Me, devices: string[] = [], state: PersonView['state'] = 'active'): PersonView {
  return {
    ...who,
    state,
    devices,
    passkeys: state === 'invited' ? 0 : 1,
    invited_by: state === 'active' ? null : ana.id,
    expires_at_ms: state === 'invited' ? Date.now() + 7 * 24 * 3600 * 1000 : null,
    me: false,
  };
}

export function startingPeople(): PersonView[] {
  return [
    view(ana, office.status.node ? [office.status.node] : []),
    view(bo, [boDevice.node]),
    view(cy),
    view({ id: 'p-di', name: 'Di', role: 'member' }, [], 'invited'),
  ];
}

const challenge = (): string => randomBytes(32).toString('base64url');
const authority = (change: string): unknown => ({
  change,
  public_key: { challenge: challenge(), rpId: 'localhost', allowCredentials: [], userVerification: 'required' },
  expires_at_ms: Date.now() + 5 * 60 * 1000,
});

/** One request the page made, as the test checks it. */
export type Call = { readonly method: string; readonly path: string; readonly body: unknown };

/** What the mocked server knows and answers; tests change it as they go. */
export class Console {
  signedIn: Me | null = null;
  people: PersonView[] = startingPeople();
  calls: Call[] = [];
  /** Answers for one path, used once each before the default. */
  readonly once = new Map<string, { status: number; json: unknown }[]>();

  boot(): Boot {
    return {
      boot: office.boot,
      seq: SEQ,
      server_time_ms: Date.now(),
      status: { ...office.status, me: this.signedIn, mode: 'public' } as Boot['status'],
      peers: office.peers,
      traffic_tail: [],
      pending: office.pending,
      versions: office.status.versions,
      last_tick: null,
    };
  }

  /** Requests to `path`, oldest first. */
  sent(path: string): Call[] {
    return this.calls.filter((c) => c.path === path);
  }

  failOnce(path: string, status: number, code: string, message = code): void {
    const list = this.once.get(path) ?? [];
    list.push({ status, json: { code, message } });
    this.once.set(path, list);
  }
}

const unauthorized = { status: 401, json: { code: 'unauthorized', message: 'sign in' } };

async function answer(server: Console, route: Route): Promise<void> {
  const request = route.request();
  const url = new URL(request.url());
  const path = url.pathname;
  const body: unknown = request.postData() ? request.postDataJSON() : null;
  server.calls.push({ method: request.method(), path, body });
  const queued = server.once.get(path)?.shift();
  if (queued) return route.fulfill(queued);
  const reply = (json: unknown, status = 200): Promise<void> => route.fulfill({ status, json });

  switch (path) {
    case '/api/boot':
      return server.signedIn ? reply(server.boot()) : route.fulfill(unauthorized);
    case '/api/status':
      return reply(server.signedIn ? server.boot().status : { mode: 'public', me: null });
    case '/api/traffic':
      return reply([]);
    case '/api/pending':
      return reply(office.pending);
    case '/api/setup/begin':
      return reply({
        public_key: {
          rp: { id: 'localhost', name: 'kith' },
          user: { id: 'cC1hbmE', name: 'Ana', displayName: 'Ana' },
          challenge: challenge(),
          pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
          excludeCredentials: [],
          authenticatorSelection: { residentKey: 'required', requireResidentKey: true, userVerification: 'required' },
          attestation: 'none',
        },
      });
    case '/api/setup/finish':
      server.signedIn = ana;
      return reply({ person: { ...view(ana), me: true }, authority: authority('authorize_console') });
    case '/api/setup/recovery':
      return reply({ words: WORDS });
    case '/api/authority/begin':
      return reply(authority('add_account'));
    case '/api/authority/finish':
      return reply({ state: 'accepted', op: null });
    case '/api/login/begin':
      return reply({ public_key: { challenge: challenge(), rpId: 'localhost', allowCredentials: [], userVerification: 'required' } });
    case '/api/login/finish':
      server.signedIn = ana;
      return reply({ mode: 'public', person: { ...view(ana), me: true }, network: { id: network.id, name: network.name } });
    case '/api/logout':
      server.signedIn = null;
      return route.fulfill({ status: 204 });
    case '/api/people':
      if (request.method() === 'POST') {
        const { name, role } = body as { name: string; role: Me['role'] };
        const person = view({ id: `p-${name.toLowerCase()}`, name, role }, [], 'invited');
        server.people = [...server.people, person];
        return reply({
          person,
          link: `https://vpn.example.com/register/${person.id}-token`,
          expires_at_ms: person.expires_at_ms,
          qr_svg: null,
        });
      }
      return reply(server.people.map((p) => ({ ...p, me: p.id === server.signedIn?.id })));
    case '/api/device-codes':
      return reply({
        code: 'K7Q4XM',
        expires_at_ms: Date.now() + 10 * 60 * 1000,
        link: 'https://vpn.example.com/j/K7Q4XM',
        command: 'kith join https://vpn.example.com/j/K7Q4XM',
        qr_svg: null,
        person: server.signedIn?.id ?? '',
      });
    default:
      if (path.startsWith('/api/setup/')) return reply({ network: null, expires_at_ms: Date.now() + 30 * 60 * 1000 });
      return reply({ code: 'not_found', message: path }, 404);
  }
}

export const WORDS = ['amber', 'river', 'falcon', 'quiet', 'lantern', 'meadow', 'copper', 'orbit', 'willow', 'harbor', 'ember', 'cedar'];

/**
 * Replaces EventSource with one the test drives: it opens at once and ticks every second,
 * so the page stays live, and `window.__stream(name, data)` sends an event.
 */
function fakeStream(): void {
  class FakeEventSource extends EventTarget {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSED = 2;
    readyState = 0;
    readonly url: string;
    readonly withCredentials = false;
    onopen = null;
    onmessage = null;
    onerror = null;
    timer: ReturnType<typeof setInterval>;
    constructor(url: string) {
      super();
      this.url = url;
      (window as unknown as { __source: FakeEventSource }).__source = this;
      setTimeout(() => {
        this.readyState = 1;
        this.dispatchEvent(new Event('open'));
        this.send('tick', { at_ms: Date.now(), traffic: [], links: [] });
      }, 0);
      this.timer = setInterval(() => this.send('tick', { at_ms: Date.now(), traffic: [], links: [] }), 1000);
    }
    send(type: string, data: unknown): void {
      if (this.readyState === 2) return;
      this.dispatchEvent(new MessageEvent(type, { data: JSON.stringify(data) }));
    }
    close(): void {
      this.readyState = 2;
      clearInterval(this.timer);
    }
  }
  Object.assign(window, {
    EventSource: FakeEventSource,
    __stream: (type: string, data: unknown) =>
      (window as unknown as { __source?: FakeEventSource }).__source?.send(type, data),
  });
}

/** Sends a journal entry down the fake stream, numbered after the snapshot. */
type Body<E> = E extends unknown ? Omit<E, 'seq' | 'at_ms'> : never;
type EntryBody = Body<Entry>;
let nextSeq = SEQ + 1;
export async function push(page: Page, entry: EntryBody): Promise<void> {
  const full = { seq: nextSeq++, at_ms: Date.now(), ...entry };
  await page.evaluate((e) => (window as unknown as { __stream: (t: string, d: unknown) => void }).__stream('entry', e), full);
}

/** A passkey for `localhost` already on the virtual authenticator, as after setup. */
function residentKey(): Record<string, unknown> {
  const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' });
  return {
    credentialId: randomBytes(16).toString('base64'),
    isResidentCredential: true,
    rpId: 'localhost',
    privateKey: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
    userHandle: Buffer.from('p-ana').toString('base64'),
    signCount: 0,
  };
}

interface Fixtures {
  server: Console;
  /** Chrome's virtual platform authenticator; `withKey` adds a passkey for localhost. */
  authenticator: { withKey(): Promise<void> };
}

export const test = base.extend<Fixtures>({
  // eslint-disable-next-line no-empty-pattern
  server: async ({}, use) => {
    await use(new Console());
  },
  authenticator: async ({ page }, use) => {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('WebAuthn.enable');
    const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    });
    await use({
      withKey: async () => {
        await cdp.send('WebAuthn.addCredential', {
          authenticatorId,
          credential: residentKey() as never,
        });
      },
    });
  },
  page: async ({ page, server }, use) => {
    const problems: string[] = [];
    page.on('pageerror', (error) => problems.push(error.message));
    await page.addInitScript(fakeStream);
    await page.route('**/api/**', (route) => answer(server, route));
    await use(page);
    expect(problems, 'no uncaught errors').toEqual([]);
  },
});

export { expect };

/** True when a base64url string has none of base64's other characters or padding. */
export function isBase64url(value: unknown): boolean {
  return typeof value === 'string' && /^[A-Za-z0-9_-]+$/.test(value);
}
