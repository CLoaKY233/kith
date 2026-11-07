<!--
@component
DeviceCode (docs/plans/console.md, flow 4): adding a computer through the Console. Makes a
6-character code for the signed-in person and shows it large, with the one command to run
on the new computer. The code works once, for 10 minutes; once it lapses, `Make a new
code`. It follows the activity stream and says `dev-box joined · Ana's device` when a
computer joins with it.
Props:
  - networkId: string.
  - person: Me, whose device it will be.
  - onShow?: (device: string) => void, `Show device` after it joined.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { announce } from '../lib/announce';
  import { errorText } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import { checkCode, clock } from '../lib/format';
  import { actions, live, onEntry } from '../lib/live/store.svelte';
  import type { DeviceCode, Me } from '../lib/types';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';

  let {
    networkId,
    person,
    onShow,
  }: { networkId: string; person: Me; onShow?: (device: string) => void } = $props();

  let code = $state.raw<DeviceCode | null>(null);
  let pending = $state(false);
  let error = $state<string | null>(null);
  let expired = $state(false);
  let joined = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function make(): Promise<void> {
    if (pending) return;
    pending = true;
    error = null;
    const handle = await actions.deviceCode(person.id).done;
    pending = false;
    if (!handle.result) {
      if (handle.error) error = errorText(handle.error);
      return;
    }
    code = handle.result;
    expired = false;
    joined = null;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => (expired = joined === null), Math.max(0, handle.result.expires_at_ms - live.now()));
  }

  onMount(() => {
    void make();
    const stop = onEntry((entry) => {
      if (code === null || joined !== null || expired) return;
      if (entry.kind !== 'device_joined' || entry.network !== networkId) return;
      joined = entry.name;
      announce(copyConsole.deviceCode.joined(entry.name, person.name));
    });
    return () => {
      stop();
      if (timer !== null) clearTimeout(timer);
    };
  });
</script>

<div class="body">
  <p>{copyConsole.deviceCode.forPerson(person.name)}</p>
  <div class="step">
    <p class="label">{copy.addDevice.notInstalled}</p>
    <CopyField value={copy.addDevice.installCommand} label={copy.addDevice.notInstalled} />
  </div>

  {#if joined !== null}
    <div class="outcome">
      <p>{copyConsole.deviceCode.joined(joined, person.name)}</p>
      {#if onShow}
        <Button variant="secondary" onclick={() => onShow?.(joined ?? '')}>{copy.join.showDevice}</Button>
      {/if}
    </div>
  {:else if code && !expired}
    <div class="step">
      <p class="label">{copyConsole.deviceCode.codeLabel}</p>
      <p class="code">
        <span aria-hidden="true">{checkCode(code.code)}</span>
        <span class="sr-only">{copy.spoken.checkCode(code.code)}</span>
      </p>
      <p class="quiet">{copyConsole.deviceCode.expires(clock(code.expires_at_ms))}</p>
    </div>
    <div class="step">
      <p class="label">{copyConsole.deviceCode.runThis}</p>
      <CopyField value={code.command} label={copyConsole.deviceCode.runThis} />
    </div>
    <p class="waiting"><span class="ring" aria-hidden="true"></span>{copyConsole.deviceCode.waiting}</p>
  {:else if expired}
    <p class="quiet">{copyConsole.deviceCode.expired}</p>
    <div><Button variant="primary" {pending} onclick={make}>{copyConsole.deviceCode.newCode}</Button></div>
  {:else if error}
    <p class="problem" role="alert">{error}</p>
    <div><Button {pending} onclick={make}>{copyConsole.deviceCode.newCode}</Button></div>
  {/if}
</div>

<style>
  .body,
  .step {
    display: grid;
    gap: var(--s-4);
  }
  .step {
    gap: var(--s-1);
  }
  .label {
    font-weight: 600;
  }
  .code {
    justify-self: start;
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-2);
    background: var(--sunken);
    font: var(--t-display);
    font-family: var(--ff-mono);
    letter-spacing: var(--t-code-tracking);
    user-select: all;
  }
  .quiet {
    color: var(--ink-3);
  }
  .problem {
    color: var(--danger);
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-2);
  }
  .ring {
    flex: none;
    width: var(--spinner);
    height: var(--spinner);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid var(--ink-3);
  }
  .outcome {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
</style>
