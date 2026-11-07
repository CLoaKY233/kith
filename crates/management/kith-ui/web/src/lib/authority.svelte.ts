import { accounts } from './accounts';
import { passkeyText } from './passkeyText';
import type { AuthorityChange } from './types';
import { usePasskey, type RequestOptionsJSON } from './webauthn';

/**
 * One authority change (a role, removing someone, a new admin), signed by the person's
 * passkey. The server's challenge is fetched as soon as the change is known, so the prompt
 * can open straight from the person's click: Safari only shows it inside a user gesture.
 */
export class AuthorityPrompt {
  state = $state<'idle' | 'signing' | 'done' | 'failed'>('idle');
  error = $state<string | null>(null);
  #change: AuthorityChange | null = null;
  #options: Promise<RequestOptionsJSON> | null = null;

  prepare(change: AuthorityChange): void {
    this.#change = change;
    this.state = 'idle';
    this.error = null;
    this.#fetch();
  }

  #fetch(): void {
    const change = this.#change;
    if (!change) return;
    const options = accounts.authorityBegin(change).then((answer) => answer.publicKey);
    options.catch(() => undefined);
    this.#options = options;
  }

  /** Opens the passkey prompt and submits the signature; true once the server took it. */
  async confirm(): Promise<boolean> {
    if (!this.#options) return false;
    this.state = 'signing';
    this.error = null;
    try {
      const assertion = await usePasskey(await this.#options);
      await accounts.authorityFinish(assertion);
      this.state = 'done';
      return true;
    } catch (cause) {
      this.error = passkeyText(cause);
      this.state = 'failed';
      // A challenge signs once; the next try needs a fresh one.
      this.#fetch();
      return false;
    }
  }
}
