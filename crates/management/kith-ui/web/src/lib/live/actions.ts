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
  RenameIn,
  RequestJoinIn,
  StatusView,
} from '../types.gen';
import type { ActionHandle } from '../types';
import type { Runner, RunnerHost } from './runner';

/** The store fields actions read and fill. */
export interface ActionsHost extends RunnerHost {
  readonly status: StatusView | null;
  peers: Record<string, Record<string, PeerInfo>>;
  records: Record<string, RecordView[]>;
  exits: Record<string, ExitView[]>;
  invites: Record<string, Invite[]>;
  /** Refetches /api/pending in the background; calls while one runs coalesce. */
  refreshPending(): void;
}

const enc = encodeURIComponent;

/** The journal entry for a device change in one network. */
const about =
  (kind: 'device_joined' | 'device_removed', network: string, node: string) =>
  (entry: Entry): boolean =>
    entry.kind === kind && entry.network === network && entry.node === node;

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
  const loadRecords = async (net: string): Promise<void> => {
    host.records[net] = await api.get<RecordView[]>(networkPath(net, '/records'));
  };
  const loadExits = async (net: string): Promise<void> => {
    host.exits[net] = await api.get<ExitView[]>(networkPath(net, '/exits'));
  };
  const loadInvites = async (net: string): Promise<void> => {
    host.invites[net] = await api.get<Invite[]>(networkPath(net, '/invites'));
  };
  const pending = (): Promise<void> => {
    host.refreshPending();
    return Promise.resolve();
  };

  return {
    // Reads that are not in the boot snapshot.
    loadRecords: (net: string) =>
      read<RecordView[]>(networkPath(net, '/records'), (v) => (host.records[net] = v)),
    loadExits: (net: string) =>
      read<ExitView[]>(networkPath(net, '/exits'), (v) => (host.exits[net] = v)),
    loadInvites: (net: string) =>
      read<Invite[]>(networkPath(net, '/invites'), (v) => (host.invites[net] = v)),
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
          api.post<Invite>(networkPath(net, '/invites'), {
            expires_secs: expiresSecs,
            uses,
          } satisfies InviteIn),
      }),
    revokeInvite: (net: string, id: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () => api.post<Outcome>(networkPath(net, `/invites/${enc(id)}/revoke`)),
        refresh: () => loadInvites(net),
      }),

    // Blocked connections (§7.5).
    allowBlocked: (id: string) =>
      runner.run<Outcome>({
        ledger: true,
        run: () => api.post<Outcome>(`/api/blocked/${enc(id)}/allow`, { preview: false } satisfies AllowIn),
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
    renameDevice: (net: string, node: string, name: string, basedOn: string | null = null) =>
      runner.run<Outcome>({
        ledger: true,
        run: () =>
          api.post<Outcome>(networkPath(net, `/devices/${enc(node)}/rename`), {
            name,
            based_on: basedOn,
          } satisfies RenameIn),
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
  } as const;
}

export type Actions = ReturnType<typeof createActions>;
