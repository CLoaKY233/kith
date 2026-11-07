// Passkeys over JSON. The Console sends WebAuthn's own JSON options, every binary field
// base64url without padding, and takes `credential.toJSON()` back. Browsers that can parse
// and write that JSON themselves do; for the rest this file turns one into the other. It
// also names why a prompt failed.

export function toBase64url(bytes: ArrayBuffer | ArrayBufferView): string {
  const view =
    bytes instanceof ArrayBuffer
      ? new Uint8Array(bytes)
      : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let binary = '';
  for (const byte of view) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Decodes base64url, with or without padding. Throws on anything else. */
export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
  if (!/^[A-Za-z0-9_-]*={0,2}$/.test(text) || text.replace(/=+$/, '').length % 4 === 1) {
    throw new TypeError('not base64url');
  }
  const plain = text.replace(/=+$/, '').replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(plain.padEnd(Math.ceil(plain.length / 4) * 4, '='));
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

type Descriptor = { readonly id: string; readonly type: string; readonly transports?: readonly string[] };

/** `PublicKeyCredentialCreationOptions` as JSON. */
export type CreationOptionsJSON = {
  readonly rp: { readonly id?: string; readonly name: string };
  readonly user: { readonly id: string; readonly name: string; readonly displayName: string };
  readonly challenge: string;
  readonly pubKeyCredParams: readonly { readonly type: string; readonly alg: number }[];
  readonly timeout?: number;
  readonly excludeCredentials?: readonly Descriptor[];
  readonly authenticatorSelection?: AuthenticatorSelectionCriteria;
  readonly attestation?: string;
  readonly extensions?: Record<string, unknown>;
};

/** `PublicKeyCredentialRequestOptions` as JSON. */
export type RequestOptionsJSON = {
  readonly challenge: string;
  readonly timeout?: number;
  readonly rpId?: string;
  readonly allowCredentials?: readonly Descriptor[];
  readonly userVerification?: string;
  readonly extensions?: Record<string, unknown>;
};

/** A new passkey, for `/api/setup/finish` and an invited person's first sign-in. */
export type RegistrationJSON = {
  readonly id: string;
  readonly rawId: string;
  readonly type: string;
  readonly authenticatorAttachment: string | null;
  readonly response: {
    readonly clientDataJSON: string;
    readonly attestationObject: string;
    readonly transports: readonly string[];
  };
  readonly clientExtensionResults: Record<string, unknown>;
};

/** A passkey's signature, for `/api/login/finish` and `/api/authority/finish`. */
export type AssertionJSON = {
  readonly id: string;
  readonly rawId: string;
  readonly type: string;
  readonly authenticatorAttachment: string | null;
  readonly response: {
    readonly clientDataJSON: string;
    readonly authenticatorData: string;
    readonly signature: string;
    readonly userHandle: string | null;
  };
  readonly clientExtensionResults: Record<string, unknown>;
};

const descriptor = (d: Descriptor): PublicKeyCredentialDescriptor => ({
  id: fromBase64url(d.id),
  type: d.type as PublicKeyCredentialType,
  ...(d.transports ? { transports: [...d.transports] as AuthenticatorTransport[] } : {}),
});

export function creationOptions(json: CreationOptionsJSON): PublicKeyCredentialCreationOptions {
  return {
    rp: { ...json.rp },
    user: { ...json.user, id: fromBase64url(json.user.id) },
    challenge: fromBase64url(json.challenge),
    pubKeyCredParams: json.pubKeyCredParams.map((p) => ({
      type: p.type as PublicKeyCredentialType,
      alg: p.alg,
    })),
    ...(json.timeout === undefined ? {} : { timeout: json.timeout }),
    ...(json.excludeCredentials ? { excludeCredentials: json.excludeCredentials.map(descriptor) } : {}),
    ...(json.authenticatorSelection ? { authenticatorSelection: { ...json.authenticatorSelection } } : {}),
    ...(json.attestation ? { attestation: json.attestation as AttestationConveyancePreference } : {}),
    ...(json.extensions ? { extensions: json.extensions as AuthenticationExtensionsClientInputs } : {}),
  };
}

export function requestOptions(json: RequestOptionsJSON): PublicKeyCredentialRequestOptions {
  return {
    challenge: fromBase64url(json.challenge),
    ...(json.timeout === undefined ? {} : { timeout: json.timeout }),
    ...(json.rpId ? { rpId: json.rpId } : {}),
    ...(json.allowCredentials ? { allowCredentials: json.allowCredentials.map(descriptor) } : {}),
    ...(json.userVerification
      ? { userVerification: json.userVerification as UserVerificationRequirement }
      : {}),
    ...(json.extensions ? { extensions: json.extensions as AuthenticationExtensionsClientInputs } : {}),
  };
}

export function registrationJSON(credential: PublicKeyCredential): RegistrationJSON {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment ?? null,
    response: {
      clientDataJSON: toBase64url(response.clientDataJSON),
      attestationObject: toBase64url(response.attestationObject),
      transports: typeof response.getTransports === 'function' ? response.getTransports() : [],
    },
    clientExtensionResults: { ...credential.getClientExtensionResults() },
  };
}

export function assertionJSON(credential: PublicKeyCredential): AssertionJSON {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment ?? null,
    response: {
      clientDataJSON: toBase64url(response.clientDataJSON),
      authenticatorData: toBase64url(response.authenticatorData),
      signature: toBase64url(response.signature),
      userHandle: response.userHandle ? toBase64url(response.userHandle) : null,
    },
    clientExtensionResults: { ...credential.getClientExtensionResults() },
  };
}

/** Why a passkey prompt gave nothing back. */
export type PasskeyProblem = 'unsupported' | 'cancelled' | 'exists' | 'failed';

/** A failed prompt, carried as an error so callers can tell it from an API failure. */
export class PasskeyError extends Error {
  readonly problem: PasskeyProblem;
  constructor(problem: PasskeyProblem, message: string = problem) {
    super(message);
    this.name = 'PasskeyError';
    this.problem = problem;
  }
}

/**
 * Maps what `navigator.credentials` threw. Browsers report a closed prompt, a timeout and
 * a refused permission all as NotAllowedError, so each reads as cancelled.
 */
export function passkeyProblem(cause: unknown): PasskeyProblem {
  if (cause instanceof PasskeyError) return cause.problem;
  const name = typeof cause === 'object' && cause !== null ? (cause as { name?: unknown }).name : null;
  switch (name) {
    case 'NotAllowedError':
    case 'AbortError':
      return 'cancelled';
    case 'InvalidStateError':
      return 'exists';
    case 'NotSupportedError':
    case 'SecurityError':
      return 'unsupported';
    default:
      return 'failed';
  }
}

/** True when this browser can make and use passkeys here (it needs a secure page). */
export function passkeysSupported(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.isSecureContext &&
    typeof window.PublicKeyCredential === 'function' &&
    typeof navigator.credentials?.create === 'function'
  );
}

async function prompt<T>(ask: () => Promise<Credential | null>, read: (c: PublicKeyCredential) => T): Promise<T> {
  if (!passkeysSupported()) throw new PasskeyError('unsupported');
  let credential: Credential | null;
  try {
    credential = await ask();
  } catch (cause) {
    throw new PasskeyError(passkeyProblem(cause), cause instanceof Error ? cause.message : String(cause));
  }
  if (credential === null) throw new PasskeyError('cancelled');
  return read(credential as PublicKeyCredential);
}

/** The browser's own JSON support, where it has it (Chrome 129, Safari 18, Firefox 119). */
type JsonParsers = Partial<
  Pick<typeof PublicKeyCredential, 'parseCreationOptionsFromJSON' | 'parseRequestOptionsFromJSON'>
>;

function parsers(): JsonParsers {
  return window.PublicKeyCredential as JsonParsers;
}

function toJSON<T>(credential: PublicKeyCredential, fallback: (c: PublicKeyCredential) => T): T {
  return typeof credential.toJSON === 'function' ? (credential.toJSON() as T) : fallback(credential);
}

/** Asks this device to make a passkey. */
export function createPasskey(options: CreationOptionsJSON): Promise<RegistrationJSON> {
  return prompt(
    () => {
      const browser = parsers();
      const publicKey =
        typeof browser.parseCreationOptionsFromJSON === 'function'
          ? browser.parseCreationOptionsFromJSON(options as unknown as PublicKeyCredentialCreationOptionsJSON)
          : creationOptions(options);
      return navigator.credentials.create({ publicKey });
    },
    (credential) => toJSON(credential, registrationJSON),
  );
}

/** Asks a passkey to sign the server's challenge. */
export function usePasskey(options: RequestOptionsJSON): Promise<AssertionJSON> {
  return prompt(
    () => {
      const browser = parsers();
      const publicKey =
        typeof browser.parseRequestOptionsFromJSON === 'function'
          ? browser.parseRequestOptionsFromJSON(options as unknown as PublicKeyCredentialRequestOptionsJSON)
          : requestOptions(options);
      return navigator.credentials.get({ publicKey });
    },
    (credential) => toJSON(credential, assertionJSON),
  );
}
