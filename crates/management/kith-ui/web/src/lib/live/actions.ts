// Every change the page can make and every read that is not in the boot snapshot (spec
// §2, §7, §16 C). Each returns an ActionHandle from the runner; screens never call fetch.
// Ledger ops are never optimistic (§14): the store changes when the journal says so.

import { api, networkPath } from '../api';
import type {
  AllowIn,
  AllowPreview,
  CreateNetworkIn,
  Entry,
  ExitIn,
  ExitView,
  GrantIn,
  ImportUseIn,
  Invite,
  InviteIn,
  JoinIn,
  JoinRequestState,
  NameCheck,
  Outcome,
  PeerInfo,
  PhoneConfig,
  PhoneConfigIn,
  PhoneCreated,
  PhoneIn,
  RecordIn,
  RecordType,
  RecordView,
  RemoveIn,
  RequestJoinIn,
  StatusView,
} from '../types.gen';
import type {
  ActionHandle,
  InvitePreview,
  OpenInvite,
  PolicyIn,
  PolicyView,
  Relay,
  RenameDeviceIn,
  ServerMode,
  WireguardImported,
  WireguardImportIn,
} from '../types';
import type { Runner, RunnerHost } from './runner';
import { tintKey } from './keys';

/** The store fields actions read and fill. */
export interface ActionsHost extends RunnerHost {
  readonly status: StatusView | null;
  peers: Record<string, Record<string, PeerInfo>>;
  records: Record<string, RecordView[]>;
  exits: Record<string, ExitView[]>;
  invites: Record<string, OpenInvite[]>;
  policies: Record<string, PolicyView>;
  relays: Record<string, Relay[]>;
  server: ServerMode | null;
  /** Refetches the boot snapshot, for changes the stream does not report. */
  resync(): void;
  /** Refetches /api/pending in the background; calls while one runs coalesce. */
  refreshPending(): void;
  /** The change tints (§8.4), when the host keeps them. */
  readonly tints?: { mark(key: string): void };
}

const enc = encodeURIComponent;

/** The journal entry for a device change in one network. */
const about =
  (kind: 'device_joined' | 'device_removed', network: string, node: string) =>
  (entry: Entry): boolean =>
    entry.kind === kind && entry.network === network && entry.node === node;

/** Passes a 2xx answer through after starting a background reload that may fail quietly. */
const afterwards =
  (reload: () => Promise<void>) =>
  <T>(value: T): T => {
    void reload().catch(() => undefined);
    return value;
  };

/** Builds `actions` over one store. */
export function createActions(host: ActionsHost, runner: Runner) {
  const read = <T>(path: string, into: (value: T) => void): ActionHandle<T> =>
    runner.run({
      read: true,
      run: async (signal) => {
        const value = await api.get<T>(path, { signal });
        into(value);
        return value;
      },
    });

  const loadPeers = async (net: string): Promise<void> => {
    const list = await api.get<PeerInfo[]>(networkPath(net, '/peers'));
    host.peers[net] = Object.fromEntries(list.map((p) => [p.node, p]));
  };
  /** Stores a records list; after the first, every name added, changed or removed tints. */
  const putRecords = (net: string, list: RecordView[]): void => {
    const before = host.records[net];
    host.records[net] = list;
    if (!before || !host.tints) return;
    const old = new Map(before.map((r) => [r.name, r]));
    for (const r of list) {
      const was = old.get(r.name);
      old.delete(r.name);
      if (!was || was.type !== r.type || was.value !== r.value) host.tints.mark(tintKey.record(net, r.name));
    }
    for (const name of old.keys()) host.tints.mark(tintKey.record(net, name));
  };
  const loadRecords = async (net: string): Promise<void> => {
    putRecords(net, await api.get<RecordView[]>(networkPath(net, '/records')));
  };
  const loadExits = async (net: string): Promise<void> => {
    host.exits[net] = await api.get<ExitView[]>(networkPath(net, '/exits'));
  };
  const loadInvites = async (net: string): Promise<void> => {
    host.invites[net] = await api.get<OpenInvite[]>(networkPath(net, '/invites'));
  };
  const loadPolicy = async (net: string): Promise<void> => {
    host.policies[net] = await api.get<PolicyView>(networkPath(net, '/policy'));
  };
  const loadRelays = async (net: string): Promise<void> => {
    host.relays[net] = await api.get<Relay[]>(networkPath(net, '/relays'));
  };
  const loadServer = async (): Promise<void> => {
    host.server = await api.get<ServerMode>('/api/server');
  };
  const pending = (): Promise<void> => {
    host.refreshPending();
    return Promise.resolve();
  };

  return {
    // Reads that are not in the boot snapshot.
    loadRecords: (net: string) =>
      read<RecordView[]>(networkPath(net, '/records'), (v) => putRecords(net, v)),
    loadExits: (net: string) =>
      read<ExitView[]>(networkPath(net, '/exits'), (v) => (host.exits[net] = v)),
    loadInvites: (net: string) =>
      read<OpenInvite[]>(networkPath(net, '/invites'), (v) => (host.invites[net] = v)),
    loadPolicy: (net: string) =>
      read<PolicyView>(networkPath(net, '/policy'), (v) => (host.policies[net] = v)),
    loadRelays: (net: string) =>
      read<Relay[]>(networkPath(net, '/relays'), (v) => (host.relays[net] = v)),
    loadServer: () => read<ServerMode>('/api/server', (v) => (host.server = v)),
    /** §2.14. Only the newest check per field counts: older ones settle as `idle`. */
    validateName: (net: string | null, name: string) =>
      runner.run<NameCheck>({
        read: true,
        latest: `validate:${net ?? ''}`,
        run: (signal) =>
          api.get<NameCheck>(`/api/validate/name?net=${enc(net ?? '')}&name=${enc(name)}`, {
            signal,
          }),
      }),
    /** §7.5 step 2: the rule sentence and new paths, before signing. */
    previewAllow: (id: string) =>
      runner.run<AllowPreview>({
        read: true,
        run: (signal) =>
          api.post<AllowPreview>(`/api/blocked/${enc(id)}/allow`, { preview: true } satisfies AllowIn, {
            signal,
          }),
      }),

    // Networks (§7.1, §7.2).
    createNetwork: (name: string, deviceName: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api.post<Outcome>('/api/networks', { name, device_name: deviceName } satisfies CreateNetworkIn),
      }),
    join: (inviteLink: string, deviceName: string) =>
      runner.run<Outcome>({
        run: () =>
          api.post<Outcome>('/api/join', {
            invite_link: inviteLink,
            device_name: deviceName,
          } satisfies JoinIn),
      }),
    /** §7.2 step 2: which network a link joins, who sent it and until when. */
    previewInvite: (inviteLink: string) =>
      runner.run<InvitePreview>({
        read: true,
        latest: 'join-preview',
        run: (signal) =>
          api.post<InvitePreview>('/api/join/preview', { invite_link: inviteLink }, { signal }),
      }),
    requestJoin: (requestCode: string, deviceName: string) =>
      runner.run<JoinRequestState>({
        run: () =>
          api.post<JoinRequestState>('/api/join/request', {
            request_code: requestCode,
            device_name: deviceName,
          } satisfies RequestJoinIn),
      }),
    /** Tier 3 (§13): the server asks the OS first. */
    leave: (net: string) =>
      runner.run<Outcome>({
        ledger: true,
        tier3: true,
        run: () => api.post<Outcome>(networkPath(net, '/leave')),
        match: (entry) => entry.kind === 'device_removed' && entry.network === net && entry.node === host.status?.node,
      }),

    // Join requests (§7.3). A 409 means another admin settled it first.
    approveJoin: (net: string, node: string, name: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () => api.post<Outcome>(networkPath(net, `/joins/${enc(node)}/approve`), { name }),
        match: about('device_joined', net, node),
        refresh: pending,
      }),
    denyJoin: (net: string, node: string) =>
      runner.run<Outcome>({
        run: () => api.post<Outcome>(networkPath(net, `/joins/${enc(node)}/deny`)),
        refresh: pending,
      }),

    // Invites (§7.4).
    createInvite: (net: string, expiresSecs: number, uses: number | null) =>
      runner.run<Invite>({
        ledger: true,
        run: () =>
          api
            .post<Invite>(networkPath(net, '/invites'), {
              expires_secs: expiresSecs,
              uses,
            } satisfies InviteIn)
            .then(afterwards(() => loadInvites(net))),
        refresh: () => loadInvites(net),
      }),
    revokeInvite: (net: string, id: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(networkPath(net, `/invites/${enc(id)}/revoke`))
            .then(afterwards(() => loadInvites(net))),
        refresh: () => loadInvites(net),
      }),

    // Blocked connections (§7.5).
    allowBlocked: (id: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(`/api/blocked/${enc(id)}/allow`, { preview: false } satisfies AllowIn)
            .then(afterwards(pending)),
        refresh: pending,
      }),

    // Phones (§7.6). Creating the config is the flow's only ledger write.
    createPhone: (net: string, name: string, tags: string[]) =>
      runner.run<PhoneCreated>({
        ledger: true,
        run: () => api.post<PhoneCreated>(networkPath(net, '/phones'), { name, tags } satisfies PhoneIn),
        match: (entry, created) =>
          entry.kind === 'device_joined' && entry.network === net && entry.node === created.node,
      }),
    /** Tier 3 (§13): revealing the private config asks the OS first. */
    phoneConfig: (net: string, node: string, everything: boolean) =>
      runner.run<PhoneConfig>({
        tier3: true,
        run: () =>
          api.post<PhoneConfig>(networkPath(net, `/phones/${enc(node)}/config`), {
            everything,
          } satisfies PhoneConfigIn),
      }),

    // Names (§6.6). `basedOn` is the op the screen showed; a 409 names who changed it.
    setRecord: (net: string, name: string, type: RecordType, value: string, basedOn: string | null = null) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api.post<Outcome>(networkPath(net, '/records'), {
            name,
            type,
            value,
            based_on: basedOn,
          } satisfies RecordIn),
        refresh: () => loadRecords(net),
      }),
    deleteRecord: (net: string, name: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () => api.post<Outcome>(networkPath(net, `/records/${enc(name)}/delete`)),
        refresh: () => loadRecords(net),
      }),

    // Exits (§7.8). Optimistic in the screen; the store keeps the server's truth and the
    // handle is seen when the daemon's exit_changed entry arrives. `null` is no exit.
    useExit: (net: string | null, node: string | null) =>
      runner.run<Outcome>({
        run: () => api.post<Outcome>('/api/exit', { network: net, node } satisfies ExitIn),
        match: (entry) => entry.kind === 'exit_changed',
      }),
    grantExit: (net: string, node: string, allowed: boolean) =>
      runner.run<Outcome>({
        ledger: true,
        run: () => api.post<Outcome>(networkPath(net, `/exits/${enc(node)}`), { allowed } satisfies GrantIn),
        refresh: () => loadExits(net),
      }),

    // Devices (§7.9, §7.11). Removing this device is tier 3.
    renameDevice: (net: string, node: string, name: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(networkPath(net, `/devices/${enc(node)}/rename`), {
              name,
            } satisfies RenameDeviceIn)
            .then(afterwards(() => loadPeers(net))),
        refresh: () => loadPeers(net),
      }),
    removeDevice: (net: string, node: string, lost: boolean) =>
      runner.run<Outcome>({
        ledger: true,
        tier3: node === host.status?.node,
        run: () =>
          api.post<Outcome>(networkPath(net, `/devices/${enc(node)}/remove`), { lost } satisfies RemoveIn),
        match: about('device_removed', net, node),
        refresh: () => loadPeers(net),
      }),

    // Access (M1.1). A 400 carries the parser's line and column in its message.
    setPolicy: (net: string, toml: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(networkPath(net, '/policy'), { toml } satisfies PolicyIn)
            .then(afterwards(() => loadPolicy(net))),
        refresh: () => loadPolicy(net),
      }),

    // Relays. `node` is the device that runs the relay, or null for none in the network.
    addRelay: (net: string, name: string, address: string, node: string | null) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(networkPath(net, '/relays'), { name, address, node } satisfies Relay)
            .then(afterwards(() => loadRelays(net))),
        refresh: () => loadRelays(net),
      }),
    removeRelay: (net: string, name: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api
            .post<Outcome>(networkPath(net, `/relays/${enc(name)}/remove`))
            .then(afterwards(() => loadRelays(net))),
        refresh: () => loadRelays(net),
      }),

    // This device as an exit or relay for its networks.
    setServer: (mode: ServerMode) =>
      runner.run<Outcome>({
        run: () =>
          api.post<Outcome>('/api/server', mode satisfies ServerMode).then((value) => {
            host.server = mode;
            void loadServer().catch(() => undefined);
            host.resync();
            return value;
          }),
        refresh: loadServer,
      }),

    // A wg-quick config becomes a network of its own, whose peer can be the exit.
    importWireguard: (name: string, confText: string) =>
      runner.run<WireguardImported>({
        run: () =>
          api
            .post<WireguardImported>('/api/wireguard/import', {
              name,
              conf_text: confText,
            } satisfies WireguardImportIn)
            .then((value) => {
              host.resync();
              return value;
            }),
      }),
    // An imported network: send its traffic through it, stop, or remove it (tier 3).
    useImport: (network: string, active: boolean) =>
      runner.run<Outcome>({
        run: () =>
          api
            .post<Outcome>(`/api/wireguard/${enc(network)}/use`, { active } satisfies ImportUseIn)
            .then((value) => {
              host.resync();
              return value;
            }),
      }),
    removeImport: (network: string) =>
      runner.run<Outcome>({
        tier3: true,
        run: () =>
          api.post<Outcome>(`/api/wireguard/${enc(network)}/remove`).then((value) => {
            host.resync();
            return value;
          }),
      }),
  } as const;
}

export type Actions = ReturnType<typeof createActions>;
