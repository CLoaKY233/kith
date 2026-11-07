// The Console's strings (docs/plans/console.md): setup, sign-in, people and device codes.
// Apart from copy.ts so they load with the screens that use them, not in the first load.

import { DOT } from './copy';

const plural = (n: number, one: string, many: string): string => (n === 1 ? one : many);
const andList = (names: readonly string[]): string =>
  names.length <= 1
    ? (names[0] ?? '')
    : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1] ?? ''}`;

export const copyConsole = {
  passkey: {
    unsupported:
      "This browser can't use passkeys on this page. Open it in a current Safari, Chrome, Edge or Firefox.",
    cancelled: 'The passkey prompt closed before it finished. Nothing changed.',
    exists: 'This device already has a passkey for this Console. Sign in with it instead.',
    failed: (message: string): string => {
      const said = message.trim().replace(/\.+$/, '');
      return said === ''
        ? "The passkey couldn't finish. Try again."
        : `The passkey couldn't finish: ${said}. Try again.`;
    },
    confirming: 'Confirm with your passkey…',
  },

  setup: {
    title: 'Set up kith',
    progress: (step: number, of: number): string => `Step ${step} of ${of}`,
    steps: ['Names', 'Passkey', 'Recovery words'],
    checking: 'Checking the link…',
    namesHeading: 'Name your network and yourself',
    namesLede: 'Both can change later.',
    ownerHeading: 'Name yourself',
    ownerLede: (network: string): string => `You become the owner of ${network}.`,
    networkLabel: 'Network name',
    networkHelper: (name: string): string => `Devices will be reachable as laptop.${name}.kith.internal`,
    personLabel: 'Your name',
    personHelper: 'Shown to the people you add.',
    personEmpty: 'Give your name.',
    continue: 'Continue',
    passkeyHeading: 'Create your passkey',
    passkeyLede:
      'You sign in with it, and it approves every change to who is in the network. It stays on this device, behind Touch ID, Face ID or your device password.',
    createPasskey: 'Create passkey',
    creating: 'Creating…',
    confirmHeading: 'Confirm you own the network',
    confirmLede: (network: string): string =>
      `Your passkey is made. Confirm once more so it signs you into ${network} as its owner.`,
    confirm: 'Confirm with Touch ID',
    confirmingLabel: 'Confirming…',
    wordsHeading: 'Save your recovery words',
    wordsLede:
      'If every owner and admin loses their passkey, these words are the only way back in. Write them down or keep them in a password manager. They are shown once.',
    wordsLabel: 'Recovery words',
    saved: 'I saved these',
    back: 'Back',
    expired:
      'This setup link expired. Setup links last 30 minutes: make a new one on the server that runs the Console.',
    used: 'This setup link was already used. Sign in with the passkey made with it.',
    invalid: "This setup link isn't valid. Check it was copied whole.",
    signIn: 'Go to sign in',
  },

  register: {
    titlePlain: 'Join kith',
    title: (network: string): string => `Join ${network}`,
    checking: 'Checking the link…',
    lede: (name: string, inviter: string, role: string): string =>
      `Hi ${name}. ${inviter} invited you as ${role === 'member' ? 'a member' : `an ${role}`}.`,
    passkey:
      'Create a passkey to sign in. It stays on this device, behind Touch ID, Face ID or your device password.',
    madeHeading: 'Your passkey is made',
    made: (inviter: string): string => `${inviter} approves you next, with their passkey. Then sign in here.`,
    signIn: 'Go to sign in',
    unconfirmed: (inviter: string): string =>
      `This link works once ${inviter || 'whoever invited you'} confirms it with their passkey. Ask them to finish adding you, then try again.`,
    expired: 'This link expired. Links last 7 days: ask for a new one.',
    used: 'This link was already used. Sign in with the passkey made with it.',
    invalid: "This link isn't valid. Check it was copied whole.",
  },

  login: {
    title: 'Sign in to kith',
    signIn: 'Sign in with passkey',
    signingIn: 'Signing in…',
    unknown:
      "kith doesn't know this passkey. Use the one you made for this Console, or ask an admin for a new link.",
  },

  people: {
    title: 'People',
    add: 'Add person',
    you: 'you',
    addPasskey: 'Add another passkey',
    consoleActs: 'This Console can act for signed-in admins.',
    revokeConsole: 'Revoke',
    consoleRevoked: 'This Console no longer acts for anyone.',
    passkeyAdded: 'Your new passkey works now.',
    pending: "Hasn't opened their link yet",
    waiting: (role: string): string =>
      `Made a passkey. Approve them to let them sign in as ${role === 'member' ? 'a member' : `an ${role}`}.`,
    approve: 'Approve…',
    approveLabel: (name: string): string => `Approve ${name}`,
    approveTitle: (name: string): string => `Approve ${name}?`,
    approveBody: (name: string, role: string, network: string): string =>
      `${name} can then sign in to ${network} as ${role === 'member' ? 'a member' : `an ${role}`}.`,
    approved: (name: string): string => `${name} can sign in now.`,
    cancelInvite: 'Take back invite',
    inviteCancelled: (name: string): string => `${name}'s link no longer works.`,
    noDevices: 'No devices yet',
    device: (name: string, online: boolean): string => `${name}${DOT}${online ? 'online' : 'offline'}`,
    actions: (name: string): string => `Actions for ${name}`,
    askAdmin: (admins: readonly string[]): string =>
      admins.length === 0
        ? 'Only admins can add people or change roles.'
        : `Only admins can add people or change roles. Ask an admin: ${andList(admins)}.`,
    loadFailed: "Couldn't load people. Check this page is still connected, then try again.",
    retry: 'Load again',
    nameLabel: 'Name',
    nameEmpty: 'Give their name.',
    roleLabel: 'Role',
    roleHelper: {
      owner: '',
      admin: 'Can add and remove devices and people, and change names, exits and access.',
      member: 'Can use the network and see who is in it. Changes need an admin.',
    },
    joinsAsMember: 'They join as a member. Only the owner can add admins.',
    createLink: 'Create link',
    creating: 'Creating…',
    confirmAdminLede: (name: string): string =>
      `Confirm with your passkey that you are making ${name} an admin: their link works once you do. After they made their passkey, you approve them once more in People.`,
    confirmAdmin: 'Confirm with Touch ID',
    linkFor: (name: string): string => `Send this link to ${name}`,
    linkLede:
      'It works once, for 7 days. Opening it lets them create a passkey; you approve them in People, then they can sign in.',
    linkLabel: 'Sign-up link',
    done: 'Done',
    makeAdmin: 'Make admin',
    makeMember: 'Make member',
    makeAdminTitle: (name: string): string => `Make ${name} an admin?`,
    makeAdminBody: (name: string, network: string): string =>
      `${name} will be able to add and remove devices and people, and change names, exits and access in ${network}.`,
    makeMemberTitle: (name: string): string => `Make ${name} a member?`,
    makeMemberBody: (name: string, network: string): string =>
      `${name} can no longer change anything in ${network}. Their devices stay.`,
    remove: 'Remove…',
    removeTitle: (name: string): string => `Remove ${name}?`,
    removeBody: (name: string, devices: number, network: string): string =>
      devices === 0
        ? `${name} can no longer sign in.`
        : `${name} can no longer sign in, and ${plural(devices, 'their device loses', `their ${devices} devices lose`)} access to ${network} within seconds.`,
    removeConfirm: (name: string): string => `Remove ${name}`,
    passkeyNote: 'Your passkey confirms this.',
    roleChanged: (name: string, role: string): string => `${name} is now ${role === 'admin' ? 'an admin' : 'a member'}.`,
    removed: (name: string): string => `Removed ${name}.`,
  },

  /** What the Console keeps past the page's last hour, for 30 days. */
  history: {
    older: 'Show older',
    olderReading: 'Reading older events…',
    olderNone: 'Nothing older is kept.',
    daysShow: 'Show the last 30 days',
    daysReading: 'Reading…',
    daysNone: 'No traffic kept from the last 30 days.',
    daysCaption: 'Traffic by day',
    daysDay: 'Day',
    daysReceived: 'Received',
    daysSent: 'Sent',
  },

  deviceCode: {
    forPerson: (person: string): string => `This computer will be ${person}'s.`,
    codeLabel: 'Code',
    runThis: 'On the new computer, run:',
    expires: (time: string): string => `Works once, until ${time}.`,
    expired: 'This code expired.',
    newCode: 'Make a new code',
    waiting: 'Waiting for the computer to join…',
    joined: (device: string, person: string): string => `${device} joined${DOT}${person}'s device`,
  },
} as const;
