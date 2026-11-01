// Every user-facing string (spec §12), with the flows' exact copy (§6, §7, §8.5). Nothing
// else in the page writes text a person reads. Vocabulary is §12.2: device, network, exit,
// blocked connection, activity. A test rejects `!`, "successfully", "please", "oops",
// "Error:" and "Loading…".
//
// Owned by workstream D; other workstreams add keys by PR to D.

/** Thin space between a number and its unit (§3.3). */
export const THIN = ' ';
/** The separator used between facts on one line. */
export const DOT = ' · ';

const plural = (n: number, one: string, many: string): string => (n === 1 ? one : many);
const quoted = (value: string): string => `"${value}"`;

export const copy = {
  brand: 'kith',
  /** §4.1: `(n) laptop · home — kith`. */
  documentTitle: (device: string, network: string | null, unseen: number): string =>
    `${unseen > 0 ? `(${unseen}) ` : ''}${device}${network ? `${DOT}${network}` : ''} — kith`,
  skipToContent: 'Skip to content',

  nav: {
    overview: 'Overview',
    devices: 'Devices',
    activity: 'Activity',
    names: 'Names',
    exits: 'Exits',
    addDevice: 'Add device',
    settings: 'Settings',
    thisDevice: 'This device',
    network: 'Network',
    primary: 'Main',
    switchNetwork: 'Switch network',
    /** §4.4: the mobile Overview tab's accessible name. */
    overviewNeedsYou: (n: number): string => `Overview, ${n} need${n === 1 ? 's' : ''} you`,
    back: 'Back',
  },

  header: {
    search: 'Search',
    searchShortcut: '⌘K',
    searchShortcutOther: 'Ctrl K',
    needsYou: (n: number): string => `${n} need${n === 1 ? 's' : ''} you`,
    needsYouTitle: 'Needs you',
    exitChip: (exit: string): string => `Exit${DOT}${exit}`,
    exitOfflineBlocked: (exit: string): string => `${exit} offline${DOT}traffic blocked`,
    exitOfflineDirect: (exit: string): string => `${exit} offline${DOT}going direct`,
    reconnecting: 'Reconnecting…',
    staleAsOf: (time: string): string => `as of ${time}`,
  },

  role: {
    admin: 'Admin',
    member: 'Member',
    view_only: 'View only',
    removed: 'Removed',
    adminExplain: (network: string): string =>
      `This device signs changes to ${network} with its own key. Every other device checks the signature.`,
    key: (fpr: string): string => `Key ${fpr}…`,
    madeAdminBy: (by: string, date: string): string => `Made admin by ${by}${DOT}${date}`,
    memberExplain: (network: string, admins: string): string =>
      `This device can see ${network}. Changes need an admin: ${admins}.`,
    /** Joins a list of names with "or" (for admins to ask). */
    orList: (names: readonly string[]): string =>
      names.length <= 1
        ? (names[0] ?? '')
        : `${names.slice(0, -1).join(', ')} or ${names[names.length - 1] ?? ''}`,
  },

  status: {
    online: 'online',
    offline: 'offline',
    stale: 'last known',
    problem: 'problem',
    needsYou: 'needs you',
    quiet: 'quiet',
  },

  path: {
    direct: 'direct',
    via: (relay: string): string => `via ${relay}`,
    stealthVia: (relay: string): string => `stealth via ${relay}`,
    flapping: 'flapping',
    mtu: (mtu: number): string => `MTU ${mtu}`,
    reportedBy: (who: string, ago: string): string => `reported by ${who}${DOT}${ago}`,
    learnWhy: 'Learn why',
  },

  units: {
    bytesPerSec: ['B/s', 'kB/s', 'MB/s', 'GB/s', 'TB/s'] as const,
    bytes: ['B', 'kB', 'MB', 'GB', 'TB'] as const,
    ms: 'ms',
    idle: 'idle',
    none: '—',
    down: '↓',
    up: '↑',
  },

  rateBand: { idle: 'idle', light: 'light', busy: 'busy' },

  /** §5 RelativeTime. Clock and weekday forms come from format.ts. */
  time: {
    now: 'now',
    secondsAgo: (s: number): string => `${s}${THIN}s ago`,
    minutesAgo: (m: number): string => `${m}${THIN}min ago`,
    hoursAgo: (h: number): string => `${h}${THIN}h ago`,
    daysAgo: (d: number): string => `${d} ${plural(d, 'day', 'days')} ago`,
    today: 'Today',
    yesterday: 'Yesterday',
  },

  /** Accessible names spoken for numbers. */
  spoken: {
    milliseconds: (n: number): string => `${n} ${plural(n, 'millisecond', 'milliseconds')}`,
    receiving: (rate: string): string => `receiving ${rate}`,
    checkCode: (code: string): string =>
      code
        .replace(/[^0-9A-Z]/gi, '')
        .split('')
        .reduce<string[]>((parts, ch, i) => {
          if (i === 3) parts.push(',');
          parts.push(ch);
          return parts;
        }, [])
        .join(' ')
        .replace(' ,', ','),
  },

  device: {
    you: 'you',
    thisMac: 'this Mac',
    thisDevice: 'this device',
    seenAgo: (ago: string): string => `seen ${ago}`,
    notConnectedYet: 'not connected yet',
    addedBy: (who: string, ago: string): string => `added by ${who} ${ago}`,
    waitingFirst: 'waiting for first connection',
    removing: 'Removing…',
    removed: 'Removed',
    more: 'More',
    /** §5 DeviceRow: "nas, online, direct, 3 milliseconds, receiving 1.2 megabytes per second". */
    rowLabel: (parts: readonly string[]): string => parts.filter(Boolean).join(', '),
    columns: {
      status: 'Status',
      name: 'Name',
      path: 'Path',
      latency: 'Latency',
      traffic: 'Traffic',
      rate: 'Rate',
      lastSeen: 'Last seen',
      actions: 'Actions',
    },
  },

  copyField: { copy: 'Copy', copied: 'Copied' },

  checkCode: {
    label: 'Check code',
    flowLabel: (code: string): string => `Check code: ${code} — the admin will see the same code.`,
  },

  qr: {
    show: 'Show QR code',
    hide: 'Hide QR code',
    label: 'QR code',
  },

  chart: {
    ranges: { '1m': '1m', '15m': '15m', '1h': '1h' } as const,
    rangeLabel: 'Range',
    now: 'now',
    minutesBack: (m: number): string => `−${m}m`,
    relayed: 'relayed',
    showData: 'Show data',
    hideData: 'Hide data',
    sparkTip: (time: string, rate: string, peak: string): string =>
      `${time}${DOT}↓ ${rate}${DOT}peak ${peak}`,
    readout: (time: string, down: string, up: string, path: string): string =>
      `${time}${DOT}↓ ${down} ↑ ${up}${DOT}${path}`,
    windowTotals: (down: string, up: string): string =>
      `↓ ${down} ↑ ${up} in the last hour`,
    summary: (device: string, peak: string, now: string): string =>
      `Traffic for ${device}. Peak ${peak}, now ${now}.`,
    tableTime: 'Minute',
    tableReceived: 'Received',
    tableSent: 'Sent',
  },

  welcome: {
    title: 'kith',
    lede: 'A private network between your devices. No server. No account.',
    start: 'Start a network',
    startSub: 'This device becomes its first device and its admin.',
    join: 'Join a network',
    joinSub: 'You have an invite link, or someone will approve you.',
    thisDevice: (name: string, os: string): string => `This device: ${name}${DOT}${os}`,
    rename: 'Rename',
  },

  /** §6.2 Overview, headline in priority order. */
  overview: {
    paused: (until: string): string => `kith is paused until ${until}.`,
    resume: 'Resume',
    exitOfflineBlocked: (exit: string): string =>
      `Your exit, ${exit}, is offline. Traffic is blocked.`,
    exitOfflineDirect: (exit: string): string =>
      `Your exit, ${exit}, is offline. Traffic now goes directly.`,
    joinOne: (device: string, network: string): string =>
      `${device} is asking to join ${network}.`,
    joinMany: (n: number, network: string): string =>
      `${n} devices are asking to join ${network}.`,
    unreachable: (device: string, ago: string): string =>
      `${device} can't be reached. Seen ${ago}.`,
    why: 'Why?',
    onlyThis: 'Only this device so far.',
    addDevice: 'Add a device',
    allGood: (online: number, total: number): string =>
      `Everything is working. ${online} of ${total} devices online.`,
    subline: (direct: number, relayed: number, relay: string, down: string, up: string): string =>
      `${direct} direct${DOT}${relayed} via ${relay}${DOT}↓ ${down} ↑ ${up}`,
    needsYou: 'Needs you',
    viewAll: (n: number): string => `View all (${n})`,
    devices: 'Devices',
    allDevices: (n: number): string => `All ${n} devices →`,
    recent: 'Recent',
    allActivity: 'All activity →',
  },

  /** §6.3 Devices. */
  devices: {
    filter: 'Filter by name or address',
    filterLabel: 'Filter devices',
    chips: {
      all: (n: number): string => `All ${n}`,
      online: (n: number): string => `Online ${n}`,
      offline: (n: number): string => `Offline ${n}`,
      relayed: (n: number): string => `Via relay ${n}`,
      phones: (n: number): string => `Phones ${n}`,
      exits: (n: number): string => `Exits ${n}`,
    },
    list: 'List',
    map: 'Map',
    sortLabel: 'Sort',
    sortName: 'Name',
    sortTraffic: 'Traffic (frozen)',
    sortedAt: (time: string): string => `Sorted at ${time}`,
    resort: 'Re-sort',
    onlyThis: (network: string): string => `Only this device is in ${network}.`,
    addDevice: 'Add a device',
    addPhone: 'Add a phone',
    noMatch: (q: string): string => `No devices match ${quoted(q)}.`,
  },

  /** §9.2 Map. */
  map: {
    offlineLabel: (name: string, ago: string): string => `${name}${DOT}${ago}`,
    tooMany: (n: number): string => `${n} devices — showing as a table.`,
    showAllLinks: 'Show all links',
    zoomIn: 'Zoom in',
    zoomOut: 'Zoom out',
    fit: 'Fit',
    openDevice: 'Open device',
    from: 'From',
    to: 'To',
    path: 'Path',
    latency: 'Latency',
    nodeLabel: (parts: readonly string[]): string => parts.filter(Boolean).join(', '),
  },

  /** §6.4 Device detail. */
  detail: {
    editName: 'Rename',
    kindOs: (kind: string, os: string): string => `${kind}${DOT}${os}`,
    fqdn: (name: string, network: string): string => `${name}.${network}.kith.internal`,
    path: 'Path',
    traffic: 'Traffic',
    names: 'Names pointing here',
    history: 'History',
    allHistory: 'All →',
    rename: 'Rename',
    useAsExit: 'Use as exit',
    allowAsExit: 'Allow as exit…',
    remove: (network: string): string => `Remove from ${network}…`,
    leave: (network: string): string => `Leave ${network}…`,
    askAdmin: 'Ask an admin…',
    close: 'Close',
  },

  /** §12.4 Verdicts. */
  verdict: {
    direct: (rtt: string): string => `Reachable directly${DOT}${rtt}`,
    relayed: (relay: string, rtt: string): string => `Reachable through ${relay}${DOT}${rtt}.`,
    offlineRecent: (ago: string): string =>
      `Not reachable right now. Seen ${ago}. It may be asleep or switching networks.`,
    offlineLong: (days: number): string => `Not seen for ${days} ${plural(days, 'day', 'days')}.`,
    smallMtu: (mtu: number): string =>
      `Reachable, but large transfers may stall (path MTU ${mtu}).`,
    blocked: (to: string, port: number): string =>
      `Reachable, but policy blocks this device → ${to} on port ${port}.`,
    allowThis: 'Allow this…',
    quiet: `Online${DOT}quiet`,
  },

  /** §6.5 Activity. */
  activity: {
    kinds: {
      all: 'All',
      changes: 'Changes',
      joins: 'Joins',
      paths: 'Paths',
      blocked: 'Blocked',
      logs: 'Logs',
    },
    device: 'Device',
    anyDevice: 'Any device',
    search: 'Search activity',
    gap: (from: string, to: string): string =>
      `Some events from ${from} to ${to} aren't available.`,
    cap: "Older events aren't kept in this view.",
    newer: (n: number): string => `${n} new ↑`,
    signedBy: (who: string): string => `signed by ${who}`,
    reportedBy: (who: string): string => `reported by ${who}`,
    flaps: (device: string, n: number, since: string): string =>
      `${device} switched paths ${n} times since ${since}`,
    levels: { info: 'INF', warn: 'WRN', error: 'ERR', debug: 'DBG', trace: 'TRC' },
    empty: 'Nothing has happened here yet.',
    notifyCardTitle: 'Get told when something needs you?',
    notifyCardBody:
      'kith can notify you when a device asks to join or your exit goes offline, even when this tab is in the background.',
    notifyOn: 'Turn on notifications',
    notifyNotNow: 'Not now',
  },

  /** §12.3 Activity templates. */
  entry: {
    peerOnline: (device: string, path: string, rtt: string | null): string =>
      `${device} came online${DOT}${path}${rtt ? `${DOT}${rtt}` : ''}`,
    peerOffline: (device: string): string => `${device} went offline`,
    pathRelayed: (device: string, relay: string, was: string): string =>
      `${device} now relayed through ${relay} (was ${was})`,
    pathDirect: (device: string, was: string): string => `${device} now direct (was ${was})`,
    pathStealth: (device: string, relay: string, was: string): string =>
      `${device} now stealth through ${relay} (was ${was})`,
    endpointChanged: (device: string): string => `${device} changed network`,
    deviceJoined: (device: string, by: string): string => `${device} joined${DOT}approved by ${by}`,
    deviceJoinedPlain: (device: string): string => `${device} joined`,
    deviceRemoved: (device: string, by: string): string => `${device} removed by ${by}`,
    deviceRemovedPlain: (device: string): string => `${device} was removed`,
    exitChanged: (exit: string): string => `This device now uses ${exit} as exit`,
    exitCleared: 'This device now uses its own connection',
    blocked: (from: string, to: string, port: string, n: number): string =>
      `Blocked ${from} → ${to}${port}${n > 1 ? `${DOT}${n} times` : ''}`,
    dropped: (reason: string, n: number): string =>
      `${n} ${plural(n, 'packet', 'packets')} blocked (${reason})`,
    joinRequested: (device: string, network: string): string =>
      `${device} asked to join ${network}`,
  },

  /** §6.6 Names. */
  names: {
    add: 'Add name',
    name: 'Name',
    value: 'Points to',
    more: 'More',
    type: 'Type',
    txt: 'Text record',
    target: (value: string): string => `→ ${value}`,
    setBy: (who: string): string => `set by ${who}`,
    empty:
      'No names yet. Give a device a name like nas.home so others can reach it without its address.',
    remove: 'Remove name',
    suffix: (network: string): string => `.${network}`,
  },

  /** §6.7 Exits. */
  exits: {
    heading: 'Send internet traffic through',
    none: `No exit${DOT}this device's own connection`,
    option: (exit: string, rtt: string): string => `${exit}${DOT}${rtt}`,
    noAccess: (exit: string): string => `You don't have access to ${exit}.`,
    allow: 'Allow',
    askAdmin: 'Ask an admin',
    onNetwork: 'Exits on the network',
    access: 'Has access',
    inUse: 'Using it now',
    allowDevice: 'Allow device…',
    stopOffering: 'Stop offering…',
    switching: 'Switching…',
    done: (exit: string): string => `Your internet now goes through ${exit}.`,
    doneNone: "Your internet now goes through this device's own connection.",
    failed: (exit: string, current: string): string =>
      `Couldn't switch: ${exit} isn't reachable right now. You're still using ${current}.`,
    failedNone: (exit: string): string =>
      `Couldn't switch: ${exit} isn't reachable right now. You're still using this device's own connection.`,
    noExitYet: 'No exit yet. Set one up in Exits →',
  },

  /** §6.8 Settings. */
  settings: {
    appearance: 'Appearance',
    theme: 'Theme',
    themes: { system: 'System', light: 'Light', dark: 'Dark' },
    compact: 'Compact rows',
    shortcuts: 'Keyboard shortcuts',
    notifications: 'Notifications',
    notificationsSwitch: 'Notifications',
    privacy: 'Privacy',
    shareLinks: (network: string): string => `Share link reports with ${network}`,
    shareLinksHelp:
      'Others see how this device connects (direct or relayed, latency), not what it sends.',
    network: 'Network',
    admins: 'Admins',
    invites: 'Invites',
    revoke: 'Revoke',
    danger: 'Danger zone',
    leave: (network: string): string => `Leave ${network}…`,
    delete: (network: string): string => `Delete ${network}…`,
    typeToConfirm: (network: string): string => `Type ${network} to confirm`,
  },

  /** §6.8 This device. */
  thisDevice: {
    title: 'This device',
    name: 'Name',
    keys: 'Keys',
    fingerprint: 'Fingerprint',
    roles: 'Role in each network',
    versions: 'Versions',
    ui: 'kith-ui',
    daemon: 'kith',
    log: 'Daemon log',
    copyLog: 'Copy redacted log',
  },

  /** §6.9 Gates. */
  gate: {
    downTitle: "kith isn't running on this device.",
    downBody: 'Start it, then this page continues by itself.',
    downCommand: 'sudo kith up',
    checking: 'Checking every 2 seconds…',
    groupTitle: "kith is running, but this user can't talk to it.",
    groupLinux: 'sudo usermod -aG kith $USER',
    groupMac: 'sudo dseditgroup -o edit -a $USER -t user kith',
    groupThen: 'Then log out and back in.',
    skew: (ui: string, daemon: string): string =>
      `kith-ui ${ui} can't talk to kith ${daemon}. Update kith on this device.`,
    authTitle: 'This page needs a fresh link.',
    authBody:
      'For safety, kith only opens for the person at this computer. Run this in a terminal:',
    authCommand: 'kith ui',
    authOnce: 'The link it opens works once.',
    authUsed: 'This link was already used. Links work once.',
    authExpired: 'This link expired. They last 2 minutes.',
    restarted: 'kith-ui restarted. Run kith ui to continue.',
  },

  /** §8.5 Connection states. */
  conn: {
    reconnecting: 'Reconnecting…',
    staleActions: "Can't change anything until kith is back.",
    daemonDown: (time: string): string =>
      `kith isn't running on this device. Showing what was true at ${time}.`,
    howToStart: 'How to start it',
    uiGone: 'This page lost its connection to kith-ui. Run kith ui to reopen it.',
    unauth: 'kith-ui restarted. Run kith ui to continue.',
    reset: (time: string): string => `kith restarted at ${time}. Everything is up to date.`,
    daemonLost: 'kith stopped on this device.',
    daemonBack: 'kith is running again.',
  },

  /** §8.6 action progress. */
  action: {
    signing: 'Signing…',
    spreading: (n: number, m: number): string => `Signed. Spreading${DOT}${n} of ${m} devices`,
    savedHere: 'Saved on this device',
    savedWaiting: 'Saved on this device. Waiting for other devices to hear about it.',
    awaitingOsAuth: 'Waiting for confirmation on this computer…',
    undo: 'Undo',
    dismiss: 'Dismiss',
    cancel: 'Cancel',
    close: 'Close',
    later: 'Later',
  },

  /** §12.5 Error mapping. */
  error: {
    permissionDenied: 'Only admins can do this. This device is a member.',
    notFound: (device: string): string => `${device} is no longer in this network.`,
    conflict: (who: string): string => `${who} changed this a moment ago. Showing the latest.`,
    nameChars: 'Names can use a–z, 0–9 and hyphens.',
    nameEmpty: 'Give it a name.',
    nameTooLong: 'Names can be up to 63 characters.',
    nameHyphenEdge: "Names can't start or end with a hyphen.",
    internal: (message: string): string =>
      `kith couldn't do this: ${message}. Details are in Logs.`,
    osAuthDenied: 'This computer did not confirm it. Nothing was signed.',
    notImplemented: "This version of kith can't do this yet.",
  },

  /** §7.1 Start a network. */
  createNetwork: {
    title: 'Start a network',
    label: 'Network name',
    initial: 'home',
    helper: (device: string, network: string): string =>
      `Devices will be reachable as ${device}.${network}.kith.internal`,
    spaces: 'Spaces become hyphens.',
    taken: (network: string): string => `You already have a network called ${network}.`,
    submit: (network: string): string => `Create ${quoted(network)}`,
    pending: 'Creating…',
    ready: (network: string): string => `${network} is ready. You're its admin.`,
    next: 'Add your next device:',
    laptop: 'Laptop or server',
    phone: 'Phone or tablet',
    later: 'Later',
  },

  /** §7.2 Join a network. */
  joinNetwork: {
    title: 'Join a network',
    label: 'Invite link',
    parsed: (network: string, by: string, minutes: number): string =>
      `Join ${network}, invited by ${by}. Link valid for ${minutes} more ${plural(minutes, 'minute', 'minutes')}.`,
    submit: (network: string): string => `Join ${network}`,
    expired: (ago: string, by: string): string =>
      `This invite expired ${ago}. Ask ${by} for a new one.`,
    used: 'This invite has already been used. Ask for a new one.',
    trying: (network: string, secs: number): string =>
      `Can't reach any device in ${network} yet. Trying… (${secs}${THIN}s)`,
    stillNothing: (network: string): string =>
      `Still no answer. Is one of ${network}'s devices online?`,
    noLink: "I don't have a link — ask to join",
    request: (command: string): string => `Your request: ${command}`,
    requestCommand: (network: string, admin: string): string => `kith join ${network}@${admin}`,
    waiting: (network: string): string => `Waiting for an admin of ${network}…`,
    joined: (online: number): string => `You're in. ${online} devices online.`,
    denied: 'An admin denied this request.',
  },

  /** §7.3 Add a laptop or server. */
  addDevice: {
    title: 'Add device',
    question: 'What are you adding?',
    laptop: 'Laptop or server',
    phone: 'Phone or tablet',
    approveLive: 'Approve it live',
    sendInvite: 'Send an invite link',
    notInstalled: 'Not installed yet?',
    installCommand: 'curl -fsSL https://kith.sh/install | sh',
    runOnNew: 'On the new device, run:',
    joinCommand: (network: string, admin: string): string => `kith join ${network}@${admin}`,
    waiting: 'Waiting for a request…',
  },

  /** §5 NeedsYouCard and §7.3 join approval. */
  join: {
    wants: (device: string, network: string): string => `${device} wants to join ${network}`,
    says: 'Says:',
    seen: 'Seen:',
    seenFrom: (ip: string, key: string): string => `from ${ip}${DOT}key ${key}`,
    seenKey: (key: string): string => `key ${key}`,
    codeLabel: 'Check code',
    approveOnlyIf: 'Approve only if the new device shows this code.',
    name: 'Name',
    deny: 'Deny',
    approve: 'Approve',
    twoSameName: (device: string): string =>
      `Two devices asked to join as ${device}. Approve only the one whose code matches.`,
    online: (device: string, path: string, rtt: string): string =>
      `${device} is online${DOT}${path}${DOT}${rtt}`,
    showDevice: 'Show device',
    denied: (device: string): string => `Denied ${device}`,
    lostRace: (who: string, ago: string): string => `Approved by ${who}${DOT}${ago}`,
    deniedBy: (who: string, ago: string): string => `Denied by ${who}${DOT}${ago}`,
    expired: (device: string): string => `This request expired. Ask ${device} to try again.`,
    justNow: 'just now',
  },

  /** §7.4 Invite link. */
  invite: {
    title: 'Send an invite link',
    validFor: 'Valid for',
    validity: { hour: '1 hour', day: '1 day', week: '1 week' },
    uses: 'Uses',
    useCounts: { one: '1', five: '5', unlimited: 'Unlimited' },
    create: 'Create invite',
    link: 'Invite link',
    orRun: (link: string): string => `Or run: kith join ${link}`,
    notUsed: (time: string): string => `Not used yet${DOT}expires ${time}`,
    usedBy: (device: string, path: string): string => `Used by ${device}${DOT}online${DOT}${path}`,
    revoke: 'Revoke invite',
    revoked: 'Invite revoked',
  },

  /** §7.5 Allow a blocked connection. */
  blocked: {
    card: (from: string, to: string, port: string, n: number): string =>
      `${from} → ${to}${port}${DOT}blocked ${n}× in the last 10 min`,
    port: (port: number, service: string | null): string =>
      ` :${port}${service ? ` (${service})` : ''}`,
    ignore: 'Ignore',
    allowThis: 'Allow this…',
    rule: (from: string, to: string, proto: string, port: number): string =>
      `Add rule: ${from} may reach ${to} on ${proto} ${port}.`,
    nothingElse: 'Nothing else changes.',
    opens: (n: number, paths: string): string => `This opens ${n} new paths: ${paths}`,
    broader: 'Need something broader?',
    editPolicy: 'Edit policy',
    addRule: 'Add rule',
    spreading: (n: number, m: number): string => `Allowed by you${DOT}spreading ${n} of ${m}`,
    done: (from: string, to: string, port: number): string =>
      `${from} can reach ${to}:${port} now.`,
    undoExplain: 'Undo signs a change that removes this rule.',
    ignored: 'Hidden for a day. Still blocked.',
  },

  /** §7.6 Add a phone. */
  addPhone: {
    title: 'Add a phone',
    stepName: 'Name it',
    stepInstall: 'Install WireGuard',
    stepScan: 'Scan',
    name: 'Name',
    initial: 'phone',
    helper: (name: string, network: string): string => `Will be ${name}.${network}.kith.internal`,
    tags: 'Tags',
    tagDefault: 'phone',
    create: 'Create phone config',
    install:
      'Install the free WireGuard app (App Store · Google Play), tap +, then Scan from QR code.',
    justMine: 'Just my devices',
    everything: 'All traffic',
    justMineHelp: (network: string): string => `Reach ${network}'s devices.`,
    everythingHelp: (exit: string): string => `Everything, through ${exit}.`,
    noExit: 'No exit yet. Set one up in Exits →',
    reveal: "Show QR code (contains this phone's private key)",
    download: 'Download .conf',
    showText: 'Show config as text',
    waiting: 'Waiting for phone to connect… Turn on the tunnel in WireGuard after scanning.',
    notYet: (gateway: string): string =>
      `Not connected yet. Check the tunnel is on, and that ${gateway} is online. Phones connect through it.`,
    connected: (phone: string, gateway: string, rtt: string): string =>
      `${phone} is connected${DOT}through ${gateway}${DOT}${rtt}`,
    noInternet: 'No internet on the phone? Turn the tunnel off and on once.',
    showAgain: 'Show QR code again',
    remove: 'Remove',
    neverConnected: (phone: string): string => `${phone} never connected. Remove it?`,
  },

  /** §7.9 Remove a device. */
  remove: {
    title: (device: string, network: string): string => `Remove ${device} from ${network}?`,
    loses: (device: string): string => `${device} loses access to every device within seconds.`,
    frees: (address: string, name: string): string =>
      `Its address ${address} and the name ${name} are freed.`,
    comeBack: 'To come back it must join again.',
    lost: (device: string): string =>
      `${device} was lost or stolen — also revoke its invites and names`,
    exitUsers: (users: string, device: string): string =>
      `${users} use ${device} as their exit. Their internet will go directly.`,
    submit: (device: string): string => `Remove ${device}`,
    progress: (n: number, m: number, device: string): string =>
      `Removing… ${n} of ${m} have dropped ${device}`,
    offlineLater: (n: number): string =>
      `${n} offline ${plural(n, 'device', 'devices')} will drop it when they're back.`,
    lastAdmin: (device: string): string =>
      `${device} is the only admin. Make another device admin first.`,
    raced: (device: string, by: string): string =>
      `${device} was removed by ${by} while you were confirming. Nothing was signed.`,
    and: ' and ',
  },

  /** §7.10 Ask an admin. */
  askAdmin: {
    title: 'Ask an admin',
    onlyAdmins: (network: string, admins: string): string =>
      `Only admins can change this. Admins in ${network}: ${admins}.`,
    adminState: (name: string, online: boolean): string =>
      `${name} (${online ? 'online' : 'offline'})`,
    sendThis: 'Send them this:',
    renameAsk: (verb: string): string => `Ask an admin to ${verb}…`,
  },

  /** §7.11 Rename. */
  rename: {
    taken: (name: string, network: string, suggestion: string): string =>
      `${name} is taken in ${network}. Use ${suggestion}?`,
    use: (suggestion: string): string => `Use ${suggestion}`,
    follow: (n: number, device: string): string =>
      `${n} ${plural(n, 'name points', 'names point')} to ${device}. They'll follow the new name.`,
    concurrent: (who: string, from: string, to: string): string =>
      `${who} changed this a second ago: ${quoted(from)} → ${quoted(to)}.`,
    useTheirs: 'Use theirs',
    keepMine: 'Keep mine',
  },

  /** §7.12 Browser notifications. */
  notify: {
    join: (device: string, network: string): string => `${quoted(device)} wants to join ${network}`,
    joinBody: 'Open kith to check the code and approve.',
    exitOffline: (exit: string): string => `Your exit, ${exit}, is offline`,
    deviceOffline: (device: string): string => `${device} went offline`,
    daemonStopped: "kith isn't running on this device",
    coalesced: (n: number): string => `${n} things need you in kith`,
  },

  /** §10 Command palette and shortcuts. */
  palette: {
    placeholder: 'Search devices and actions',
    label: 'Command palette',
    empty: 'No matches.',
    groups: { devices: 'Devices', actions: 'Actions', goTo: 'Go to', networks: 'Networks' },
    addDevice: 'Add device',
    addPhone: 'Add a phone',
    newInvite: 'New invite',
    useExit: 'Use exit…',
    addName: 'Add name',
    rename: (device: string): string => `Rename ${device}`,
    copyAddress: (device: string): string => `Copy ${device} address`,
    askRename: (device: string): string => `Ask an admin to rename ${device}…`,
  },

  shortcuts: {
    title: 'Keyboard shortcuts',
    palette: 'Open the command palette',
    filter: 'Focus the filter',
    help: 'Show shortcuts',
    goOverview: 'Go to Overview',
    goDevices: 'Go to Devices',
    goActivity: 'Go to Activity',
    close: 'Close the topmost layer',
    approveDeny: 'Approve or deny the focused join request',
  },

  /** §13 friction tier 3. */
  tier3: {
    waiting: 'Waiting for confirmation on this computer…',
  },

  /** §6.3, §6.4, §12.4: Devices and Device detail, beyond the shared keys above. */
  deviceView: {
    addDeviceMobile: '+ Add device',
    filterChipsLabel: 'Show devices',
    viewLabel: 'View',
    sortBy: (by: string): string => `Sort: ${by}`,
    caret: '▾',
    listLabel: (network: string): string => `Devices in ${network}`,
    rowMenu: (device: string): string => `More for ${device}`,
    copyAddress: 'Copy address',
    copyName: 'Copy name',
    copyThis: (what: string): string => `Copy ${what}`,
    addedAgo: (ago: string): string => `added ${ago}`,
    spokenBytesPerSec: [
      'bytes per second',
      'kilobytes per second',
      'megabytes per second',
      'gigabytes per second',
      'terabytes per second',
    ] as const,
    relayed: 'relayed',
    stealth: 'stealth',
    relayNode: 'relay',
    selfMac: 'This Mac',
    selfDevice: 'This device',
    endpoint: 'Endpoint',
    pathLabel: (hops: readonly string[], latency: string | null): string =>
      `${hops.join(' to ')}${latency ? `, ${latency}` : ''}`,
    nameLabel: 'Device name',
    renamed: (from: string, to: string): string => `Renamed ${from} to ${to}`,
    totals: {
      '1m': (down: string, up: string): string => `↓ ${down} ↑ ${up} in the last minute`,
      '15m': (down: string, up: string): string => `↓ ${down} ↑ ${up} in the last 15 minutes`,
      '1h': (down: string, up: string): string => `↓ ${down} ↑ ${up} in the last hour`,
    },
    noHistory: 'Nothing has happened to this device yet.',
    detailLabel: (device: string): string => `${device} details`,
    askRemove: (device: string, network: string): string =>
      `Ask to remove ${device} from ${network}…`,
    askAllowExit: (device: string): string => `Ask to allow ${device} as exit…`,
    removeCommand: (network: string, device: string): string =>
      `kith remove ${network} ${device}`,
    allowExitCommand: (network: string, device: string): string =>
      `kith exit allow ${network} ${device}`,
    allowCommand: (network: string, from: string, to: string, proto: string, port: string): string =>
      `kith allow ${network} ${from} ${to} ${proto}${port ? ` ${port}` : ''}`,
    allowExitTitle: (device: string): string => `Allow ${device} as an exit?`,
    allowExitBody: (device: string, network: string): string =>
      `Devices in ${network} that may use it can send all their internet traffic through ${device}. Websites see ${device}'s address instead of theirs.`,
    allowExitSubmit: (device: string): string => `Allow ${device} as exit`,
    allowedExit: (device: string): string => `${device} is now an exit.`,
    usingExit: (device: string): string => `This device's internet goes through ${device}.`,
  },

  /** §12.4 Verdicts the table leaves implied: no RTT yet, offline between 10 min and a day, never seen. */
  verdictMore: {
    direct: 'Reachable directly',
    relayed: (relay: string): string => `Reachable through ${relay}.`,
    offlineSome: (ago: string): string => `Not reachable right now. Seen ${ago}.`,
    never: 'Not connected yet.',
    waitingPhone: 'Waiting for its first connection.',
  },
} as const;

export type Copy = typeof copy;
