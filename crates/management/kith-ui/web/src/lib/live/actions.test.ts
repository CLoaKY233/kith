import { afterEach, describe, expect, it, vi } from 'vitest';
import { createActions, type ActionsHost } from './actions';
import { Runner } from './runner';

type Call = { method: string; url: string; body: unknown };

function setup(answers: Record<string, unknown> = {}) {
  const calls: Call[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init: RequestInit) => {
      const body = typeof init.body === 'string' ? (JSON.parse(init.body) as unknown) : null;
      calls.push({ method: init.method ?? 'GET', url, body });
      const answer = answers[`${init.method} ${url}`] ?? {};
      return new Response(JSON.stringify(answer), { status: 200 });
    }),
  );
  const host: ActionsHost = {
    conn: 'live',
    status: null,
    peers: {},
    records: {},
    exits: {},
    invites: {},
    policies: {},
    relays: {},
    server: null,
    resync: vi.fn(),
    refreshPending: vi.fn(),
  };
  return { calls, host, actions: createActions(host, new Runner(host)) };
}

afterEach(() => vi.unstubAllGlobals());

describe('actions for the newer endpoints', () => {
  it('saves a policy and loads it back', async () => {
    const policy = { toml: 'rules = []\n', rules: [] };
    const { calls, host, actions } = setup({ 'GET /api/networks/n1/policy': policy });
    const handle = actions.setPolicy('n1', 'rules = []\n');
    await handle.done;
    await vi.waitFor(() => expect(host.policies['n1']).toEqual(policy));
    expect(calls[0]).toEqual({ method: 'POST', url: '/api/networks/n1/policy', body: { toml: 'rules = []\n' } });
  });

  it('adds and removes relays, then lists them again', async () => {
    const list = [{ name: 'main', address: 'vps.example.com:41642', node: null }];
    const { calls, host, actions } = setup({ 'GET /api/networks/n1/relays': list });
    await actions.addRelay('n1', 'main', 'vps.example.com:41642', null).done;
    await actions.removeRelay('n1', 'main').done;
    await vi.waitFor(() => expect(host.relays['n1']).toEqual(list));
    expect(calls.filter((c) => c.method === 'POST')).toEqual([
      { method: 'POST', url: '/api/networks/n1/relays', body: { name: 'main', address: 'vps.example.com:41642', node: null } },
      { method: 'POST', url: '/api/networks/n1/relays/main/remove', body: {} },
    ]);
  });

  it('renames with the name alone and revokes invites by id', async () => {
    const { calls, actions } = setup();
    await actions.renameDevice('n1', 'node1', 'storage').done;
    await actions.revokeInvite('n1', 'inv 1').done;
    const posts = calls.filter((c) => c.method === 'POST');
    expect(posts[0]).toEqual({ method: 'POST', url: '/api/networks/n1/devices/node1/rename', body: { name: 'storage' } });
    expect(posts[1]?.url).toBe('/api/networks/n1/invites/inv%201/revoke');
  });

  it('turns server mode on and imports a WireGuard config', async () => {
    const { calls, host, actions } = setup({
      'GET /api/server': { exit: true, relay: false },
      'POST /api/wireguard/import': { network: 'n2' },
    });
    await actions.setServer({ exit: true, relay: false }).done;
    expect(host.server).toEqual({ exit: true, relay: false });
    const imported = await actions.importWireguard('office', '[Interface]\n[Peer]\n').done;
    expect(imported.result).toEqual({ network: 'n2' });
    expect(host.resync).toHaveBeenCalledTimes(2);
    expect(calls.find((c) => c.url === '/api/wireguard/import')?.body).toEqual({
      name: 'office',
      conf_text: '[Interface]\n[Peer]\n',
    });
  });

  it('uses, stops and removes an imported network', async () => {
    const { calls, host, actions } = setup({
      'POST /api/wireguard/w1/use': {},
      'POST /api/wireguard/w1/remove': {},
    });
    await actions.useImport('w1', false).done;
    await actions.removeImport('w1').done;
    const posts = calls.filter((c) => c.method === 'POST');
    expect(posts.map((c) => [c.url, c.body])).toEqual([
      ['/api/wireguard/w1/use', { active: false }],
      ['/api/wireguard/w1/remove', {}],
    ]);
    expect(host.resync).toHaveBeenCalledTimes(2);
  });
});
