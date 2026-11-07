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
    namesHeading: 'Name your network and yourself',
    namesLede: 'Both can change later.',
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
    pending: "Hasn't opened their link yet",
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
      `Making ${name} an admin is signed by your passkey, so every device can check it was you.`,
    confirmAdmin: 'Confirm with Touch ID',
    linkFor: (name: string): string => `Send this link to ${name}`,
    linkLede: 'It works once. Opening it lets them create a passkey and sign in.',
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
