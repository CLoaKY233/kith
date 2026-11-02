<!--
@component
ImportWireguard: a wg-quick config (pasted, or read from a chosen .conf file) becomes a
network of its own through `POST /api/wireguard/import`; its server is then an exit this
device can pick. Once kith lists the new network, the page opens its Exits. The config holds
a private key, so the field never autocompletes or spell-checks, and nothing keeps it after
the sheet closes.
Props:
  - open: boolean, bindable. Exits maps it to `?sheet=wireguard` (Back closes it).
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { announce } from '../lib/announce';
  import { errorText } from '../lib/api';
  import { copy } from '../lib/copy';
  import { actions, live } from '../lib/live/store.svelte';
  import type { ActionHandle, WireguardImported } from '../lib/types';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { go, netHref } from '../router';
  import { isStale } from './flow';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let name = $state('');
  let conf = $state('');
  let file = $state<string | null>(null);
  let nameError = $state<string | null>(null);
  let confError = $state<string | null>(null);
  let importing = $state<ActionHandle<WireguardImported> | null>(null);
  let picker = $state<HTMLInputElement | null>(null);
  /** The imported network, until kith lists it and its Exits open. */
  let waitingFor = $state<string | null>(null);
  const stale = $derived(isStale());
  const pending = $derived(importing?.state === 'pending');
  const failed = $derived(
    importing?.state === 'failed' && importing.error ? errorText(importing.error) : null,
  );

  // Each opening starts empty: the last config's private key is not kept around.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      name = '';
      conf = '';
      file = null;
      nameError = null;
      confError = null;
      importing = null;
    });
  });

  $effect(() => {
    const id = waitingFor;
    if (id && live.status?.networks.some((n) => n.id === id)) {
      untrack(() => {
        waitingFor = null;
        go(netHref(id, 'exits'));
      });
    }
  });

  async function choose(event: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const chosen = event.currentTarget.files?.[0];
    event.currentTarget.value = '';
    if (!chosen) return;
    try {
      conf = await chosen.text();
      file = chosen.name;
      confError = null;
      if (name === '') {
        name = chosen.name
          .replace(/\.conf$/i, '')
          .toLowerCase()
          .replace(/[^a-z0-9-]+/g, '-')
          .replace(/^-+|-+$/g, '');
      }
    } catch {
      confError = copy.wgImport.fileFailed;
      announce(confError);
    }
  }

  function check(): boolean {
    const n = name.trim();
    nameError =
      n === ''
        ? copy.error.nameEmpty
        : n.length > 63
          ? copy.error.nameTooLong
          : /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(n)
            ? null
            : copy.error.nameChars;
    confError =
      conf.trim() === ''
        ? copy.wgImport.confEmpty
        : /^\s*\[Interface\]/im.test(conf) && /^\s*\[Peer\]/im.test(conf)
          ? null
          : copy.wgImport.confNotWg;
    const problem = nameError ?? confError;
    if (problem) announce(problem);
    return problem === null;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (stale || pending || !check()) return;
    const n = name.trim();
    const handle = actions.importWireguard(n, conf);
    importing = handle;
    await handle.done;
    if (handle.state !== 'accepted' || !handle.result) {
      if (failed) announce(failed);
      return;
    }
    waitingFor = handle.result.network;
    conf = '';
    open = false;
    toast.done(copy.wgImport.done(n));
  }

  const formId = $props.id();
</script>

<Sheet bind:open title={copy.wgImport.title} description={copy.wgImport.intro} closeLabel={copy.action.close}>
  <form id={formId} class="form" novalidate onsubmit={(e) => void submit(e)}>
    <Field label={copy.wgImport.name} helper={copy.wgImport.nameHelp} error={nameError}>
      {#snippet children(control)}
        <Input
          {...control}
          kind="name"
          bind:value={name}
          readonly={pending}
          oninput={() => (nameError = null)}
        />
      {/snippet}
    </Field>
    <div class="conf">
      <label class="label" for="{formId}-conf">{copy.wgImport.conf}</label>
      <p class="quiet" id="{formId}-conf-help">{file ? copy.wgImport.chosen(file) : copy.wgImport.confHelp}</p>
      <textarea
        id="{formId}-conf"
        class="text"
        rows="10"
        bind:value={conf}
        readonly={pending}
        spellcheck="false"
        autocomplete="off"
        autocapitalize="none"
        aria-invalid={confError ? 'true' : undefined}
        aria-describedby="{formId}-conf-help{confError ? ` ${formId}-conf-error` : ''}"
        oninput={() => {
          confError = null;
          file = null;
        }}
      ></textarea>
      {#if confError}<p class="error" id="{formId}-conf-error">{confError}</p>{/if}
      <div>
        <input
          bind:this={picker}
          class="sr-only"
          type="file"
          accept=".conf,text/plain"
          tabindex="-1"
          aria-hidden="true"
          onchange={(e) => void choose(e)}
        />
        <Button variant="secondary" onclick={() => picker?.click()}>{copy.wgImport.choose}</Button>
      </div>
    </div>
    {#if failed}<p class="error">{failed}</p>{/if}
  </form>

  {#snippet footer()}
    {#if stale}<p class="reason">{copy.conn.staleActions}</p>{/if}
    <Button variant="secondary" onclick={() => (open = false)}>{copy.action.cancel}</Button>
    <Button
      variant="primary"
      type="submit"
      form={formId}
      pending={pending}
      pendingLabel={copy.wgImport.pending}
      disabled={stale}
    >
      {copy.wgImport.submit}
    </Button>
  {/snippet}
</Sheet>

<style>
  .form,
  .conf {
    display: grid;
    gap: var(--s-2);
  }
  .form {
    gap: var(--s-5);
  }
  .label {
    font-weight: 600;
  }
  .text {
    width: 100%;
    padding: var(--s-3);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-1);
    font: var(--t-mono);
    resize: vertical;
  }
  .text:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .text[aria-invalid='true'] {
    border-color: var(--danger);
  }
  .quiet,
  .reason {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .reason {
    flex: 1;
    align-self: center;
  }
  .error {
    font: var(--t-small);
    color: var(--danger);
  }
</style>
