<!--
@component
Names (spec §6.6): the names set in a network, beyond each device's own. A table of
`name.home` (mono) · `→ target` · `set by laptop` (seal) · Copy · ⋯.

`Add name` opens an inline row at the top, never a modal: the name with the `.home` suffix
shown, and what it points to. The type is inferred: an IP gives A or AAAA, a host name gives
CNAME; `More` reveals a text record. A name typed bare (`nas`, `nas.home`) becomes the full
`nas.home.kith.internal`. Adding and removing are tier 0: instant, removal with Undo.
Members fill in the same row; its button becomes `Ask an admin…` with the exact command.
The row sits in the table under the column headers, its fields in the columns they fill.
Props:
  - net: string, the network's id or name (as in the URL).
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { flip } from 'svelte/animate';
  import { slide } from 'svelte/transition';
  import { announce } from '../lib/announce';
  import { copy, DOT } from '../lib/copy';
  import Icon from '../lib/icons/Icon.svelte';
  import AskAdminSheet from '../lib/kith/AskAdminSheet.svelte';
  import { tintKey } from '../lib/live/keys';
  import { actions, live } from '../lib/live/store.svelte';
  import {
    BANNER_DELAY_MS,
    COPIED_MS,
    ROW_ANIMATION_MS,
    SEEN_TIMEOUT_MS,
    SPACES_HINT_MS,
    UNDO_MS,
  } from '../lib/live/timing';
  import type { ActionHandle, ApiFailure } from '../lib/types';
  import type { NetworkSummary, Outcome, RecordType, RecordView } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import DropdownMenu, { type MenuItem } from '../lib/ui/DropdownMenu.svelte';
  import Field from '../lib/ui/Field.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import { toast } from '../lib/ui/Toast.svelte';

  let { net }: { net: string } = $props();

  function findNetwork(list: readonly NetworkSummary[], ref: string): NetworkSummary | null {
    return (
      list.find((n) => n.id === ref) ??
      list.find((n) => n.name === ref) ??
      (ref === '' ? undefined : list.find((n) => n.id.startsWith(ref))) ??
      null
    );
  }

  const network = $derived(findNetwork(live.status?.networks ?? [], net));
  const netId = $derived(network?.id ?? null);
  const netName = $derived(network?.name ?? net);
  const admin = $derived(network?.admin ?? false);
  const peers = $derived(netId ? Object.values(live.peers[netId] ?? {}) : []);
  const records = $derived(netId ? (live.records[netId] ?? null) : null);

  /** `.home.kith.internal`: what every full name in this network ends with. */
  const fullSuffix = $derived(copy.detail.fqdn('', netName));
  const label = (full: string): string =>
    full.endsWith(fullSuffix) ? full.slice(0, -fullSuffix.length) : full.replace(/\.$/, '');
  const short = (full: string): string =>
    full.endsWith(fullSuffix) ? `${label(full)}${copy.names.suffix(netName)}` : label(full);
  const escaped = $derived(netName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  /** A name as typed, without the `.home` or `.home.kith.internal` the field already shows. */
  const bare = (typed: string): string =>
    typed.replace(new RegExp(`\\.${escaped}(\\.kith\\.internal)?\\.?$`), '');

  const names = $derived(
    (records ?? []).filter((r) => !r.device).sort((a, b) => a.name.localeCompare(b.name)),
  );
  const deviceLabels = $derived(
    new Set([
      ...(records ?? []).filter((r) => r.device).map((r) => label(r.name)),
      ...peers.map((p) => p.name),
    ]),
  );

  // §8.5: after STALE plus BANNER_DELAY, nothing can change until kith is back.
  let frozen = $state(false);
  $effect(() => {
    const down = live.conn === 'stale' || live.conn === 'daemon_down' || live.conn === 'ui_gone';
    if (!down) {
      frozen = false;
      return;
    }
    const timer = setTimeout(() => (frozen = true), BANNER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  // §3.5: rows grow in and collapse out over 160 ms, at once with reduced motion.
  const reduced =
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const rowMs = reduced ? 0 : ROW_ANIMATION_MS;

  // §8.4: the store tints names added, changed or removed when the list reloads.
  let loadFailure = $state<ApiFailure | null>(null);
  async function reload(id: string): Promise<void> {
    const handle = await actions.loadRecords(id).done;
    loadFailure = handle.state === 'failed' ? handle.error : null;
  }
  $effect(() => {
    const id = netId;
    if (id !== null) untrack(() => void reload(id));
  });
  function retry(): void {
    if (netId !== null) void reload(netId);
  }
  const ready = $derived(records !== null || loadFailure !== null);

  function failure(handle: ActionHandle<Outcome>, subject: string): string {
    if (handle.state === 'conflict' && handle.conflict) {
      return copy.error.conflict(handle.conflict.resolved_by);
    }
    switch (handle.error?.code) {
      case 'permission_denied':
        return copy.error.permissionDenied;
      case 'daemon_down':
        return copy.conn.staleActions;
      case 'not_implemented':
        return copy.error.notImplemented;
      case 'not_found':
        return copy.error.notFound(subject);
      default:
        return copy.error.internal(handle.error?.message ?? '');
    }
  }

  function progress(handle: ActionHandle<Outcome>, subject: string): string {
    switch (handle.state) {
      case 'pending':
        return copy.action.signing;
      case 'awaiting_os_auth':
        return copy.action.awaitingOsAuth;
      case 'failed':
      case 'conflict':
        return failure(handle, subject);
      default:
        if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
        return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
    }
  }

  /** What a name points to, as the table shows it. */
  function target(record: RecordView): { text: string; mono: boolean; address: string | null } {
    if (record.type === 'TXT') return { text: copy.names.text(record.value), mono: false, address: null };
    if (record.type === 'CNAME') return { text: short(record.value), mono: true, address: null };
    const device = peers.find((p) => p.address === record.value);
    return device
      ? { text: device.name, mono: false, address: record.value }
      : { text: record.value, mono: true, address: null };
  }

  // ---- Add name: the inline row (§6.6) ----

  const IPV4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;
  const HOST = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/;

  function ipv6(value: string): boolean {
    try {
      return new URL(`http://[${value}]`).hostname !== '';
    } catch {
      return false;
    }
  }

  type Parsed = { type: RecordType; value: string };
  function parse(raw: string, text: boolean): Parsed | 'empty' | 'invalid' {
    const value = raw.trim();
    if (value === '') return 'empty';
    if (text) return { type: 'TXT', value };
    if (IPV4.test(value)) return { type: 'A', value };
    if (value.includes(':')) return ipv6(value) ? { type: 'AAAA', value: value.toLowerCase() } : 'invalid';
    const host = value.replace(/\.$/, '').toLowerCase();
    if (!HOST.test(host)) return 'invalid';
    if (!host.includes('.')) return { type: 'CNAME', value: copy.detail.fqdn(host, netName) };
    const netSuffix = copy.names.suffix(netName);
    if (host.endsWith(netSuffix)) {
      return { type: 'CNAME', value: copy.detail.fqdn(host.slice(0, -netSuffix.length), netName) };
    }
    return { type: 'CNAME', value: host };
  }

  let adding = $state(false);
  let nameText = $state('');
  let valueText = $state('');
  let text = $state(false);
  let more = $state(false);
  let nameError = $state<string | null>(null);
  let valueError = $state<string | null>(null);
  let nameChecked = false;
  let valueChecked = false;
  let spacesHint = $state(false);
  let spacesTimer: ReturnType<typeof setTimeout> | null = null;
  let formError = $state<string | null>(null);
  let saving = $state<ActionHandle<Outcome> | null>(null);
  const pending = $derived(saving?.state === 'pending');

  let nameRef: HTMLInputElement | null = $state(null);
  let valueRef: HTMLInputElement | null = $state(null);
  let opener: HTMLElement | null = null;

  const existing = $derived(
    names.find((r) => nameText !== '' && r.name === copy.detail.fqdn(bare(nameText), netName)) ??
      null,
  );
  const nameHelp = $derived(
    existing === null
      ? undefined
      : existing.type === 'TXT'
        ? copy.names.replacesText(short(existing.name), existing.value)
        : copy.names.replaces(short(existing.name), target(existing).text),
  );

  async function open(event: MouseEvent): Promise<void> {
    opener = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
    adding = true;
    await tick();
    nameRef?.focus();
  }

  function reset(): void {
    adding = false;
    nameText = '';
    valueText = '';
    text = false;
    more = false;
    nameError = null;
    valueError = null;
    nameChecked = false;
    valueChecked = false;
    formError = null;
    saving = null;
  }

  async function close(): Promise<void> {
    reset();
    await tick();
    (opener?.isConnected ? opener : document.getElementById(addId))?.focus();
  }

  let nameAttempt = 0;
  async function checkName(): Promise<string | null> {
    nameAttempt += 1;
    const mine = nameAttempt;
    const handle = actions.validateName(netId, bare(nameText));
    await handle.done;
    if (mine !== nameAttempt) return nameError;
    const result = handle.result;
    // Without an answer the server decides on submit.
    if (handle.state === 'failed' || result === null) return null;
    switch (result.problem) {
      case 'empty':
        return copy.error.nameEmpty;
      case 'too_long':
        return copy.error.nameTooLong;
      case 'invalid_character':
        return copy.error.nameChars;
      case 'hyphen_at_edge':
        return copy.error.nameHyphenEdge;
      default:
        return deviceLabels.has(result.name)
          ? copy.names.deviceName(`${result.name}${copy.names.suffix(netName)}`)
          : null;
    }
  }

  function checkValue(): string | null {
    const parsed = parse(valueText, text);
    if (parsed === 'empty') return copy.names.valueEmpty;
    if (parsed === 'invalid') return copy.names.valueInvalid(netName);
    return null;
  }

  // §7.1 rules for names: a capital lowercases, a space becomes a hyphen and says so.
  async function nameInput(): Promise<void> {
    const next = nameText.toLowerCase().replace(/ /g, '-');
    if (next !== nameText.toLowerCase()) {
      spacesHint = true;
      if (spacesTimer !== null) clearTimeout(spacesTimer);
      spacesTimer = setTimeout(() => (spacesHint = false), SPACES_HINT_MS);
    }
    nameText = next;
    formError = null;
    if (nameChecked) nameError = await checkName();
  }

  async function nameBlur(): Promise<void> {
    if (nameText === '' || pending) return;
    nameText = bare(nameText);
    nameChecked = true;
    nameError = await checkName();
  }

  function valueInput(): void {
    formError = null;
    if (valueChecked) valueError = checkValue();
  }

  function valueBlur(): void {
    if (valueText === '' || pending) return;
    valueChecked = true;
    valueError = checkValue();
  }

  $effect(() => {
    void text;
    if (untrack(() => valueChecked)) valueError = untrack(checkValue);
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (pending || netId === null || (frozen && admin)) return;
    nameText = bare(nameText);
    nameChecked = true;
    valueChecked = true;
    valueError = checkValue();
    nameError = await checkName();
    if (nameError) {
      nameRef?.focus();
      return;
    }
    const parsed = parse(valueText, text);
    if (valueError || typeof parsed === 'string') {
      valueRef?.focus();
      return;
    }
    if (!admin) {
      openAsk(copy.askAdmin.request.dnsSet(netName, nameText, parsed.type, parsed.value));
      return;
    }
    const id = netId;
    const full = copy.detail.fqdn(nameText, netName);
    const handle = actions.setRecord(id, nameText, parsed.type, parsed.value);
    saving = handle;
    formError = null;
    const settled = await handle.done;
    if (saving !== handle) return;
    if (settled.state === 'accepted' || settled.state === 'awaiting_os_auth') {
      follow(full, handle);
      announce(progress(handle, short(full)));
      void reload(id);
      await close();
    } else {
      formError = failure(settled, short(full));
      announce(formError);
      saving = null;
      if (settled.state === 'conflict') void reload(id);
    }
  }

  // Esc anywhere in the row reverts it, like Esc in a rename field (§7.11).
  let formRef: HTMLFormElement | null = $state(null);
  $effect(() => {
    const form = formRef;
    if (form === null) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape' || pending) return;
      event.preventDefault();
      event.stopPropagation();
      void close();
    };
    form.addEventListener('keydown', onKey);
    return () => form.removeEventListener('keydown', onKey);
  });

  // ---- Rows: copy, remove with Undo ----

  // The row's progress text (§8.6) shows while its action is under way, then gives way
  // to `set by …` again: once every device has it, or SEEN_TIMEOUT after `Waiting…`.
  let ops = $state<Record<string, ActionHandle<Outcome>>>({});

  function drop(key: string, handle: ActionHandle<Outcome>): void {
    if (ops[key] !== handle) return;
    ops = Object.fromEntries(Object.entries(ops).filter(([name]) => name !== key));
  }

  function follow(key: string, handle: ActionHandle<Outcome>): void {
    ops = { ...ops, [key]: handle };
    setTimeout(() => drop(key, handle), 2 * SEEN_TIMEOUT_MS);
  }

  $effect(() => {
    const heard = Object.entries(ops).filter(
      ([, h]) => h.state === 'seen' && (h.seen === null || h.seen.by >= h.seen.of),
    );
    if (heard.length > 0) untrack(() => heard.forEach(([key, h]) => drop(key, h)));
  });

  let copied = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;
  async function copyName(full: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(full);
    } catch {
      return;
    }
    copied = full;
    if (copiedTimer !== null) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied = null), COPIED_MS);
  }

  function restore(record: RecordView): void {
    if (netId === null) return;
    const id = netId;
    const handle = actions.setRecord(id, label(record.name), record.type, record.value);
    ops = { ...ops, [record.name]: handle };
    void handle.done.then((h) => {
      if (h.state === 'accepted' || h.state === 'awaiting_os_auth') {
        follow(record.name, handle);
        void reload(id);
      } else {
        drop(record.name, handle);
        toast.failed(failure(h, short(record.name)));
      }
    });
  }

  /** §10: focus goes back where the removed row's ⋯ was, or to Add name. */
  function refocus(removed: string, index: number): void {
    const active = document.activeElement;
    const lost =
      active === null ||
      active === document.body ||
      !active.isConnected ||
      active.closest(`[data-name="${CSS.escape(removed)}"]`) !== null;
    if (!lost) return;
    const next = names[index] ?? names[index - 1];
    const target = next
      ? document.querySelector<HTMLElement>(`[data-name="${CSS.escape(next.name)}"] .more button`)
      : document.getElementById(addId);
    target?.focus();
  }

  function remove(record: RecordView): void {
    if (netId === null || frozen || ops[record.name]?.state === 'pending') return;
    const id = netId;
    const index = names.findIndex((r) => r.name === record.name);
    const handle = actions.deleteRecord(id, label(record.name));
    ops = { ...ops, [record.name]: handle };
    void handle.done.then(async (h) => {
      if (h.state === 'accepted' || h.state === 'awaiting_os_auth') {
        toast.done(copy.names.removed(short(record.name)), {
          undo: () => restore(record),
          undoMs: UNDO_MS,
        });
        await reload(id);
        drop(record.name, handle);
        await tick();
        refocus(record.name, index);
      } else {
        drop(record.name, handle);
        toast.failed(failure(h, short(record.name)));
        if (h.state === 'conflict') void reload(id);
      }
    });
  }

  function items(record: RecordView): MenuItem[] {
    return admin
      ? [
          {
            label: copy.names.remove,
            tone: 'danger',
            disabled: frozen,
            onSelect: () => remove(record),
          },
        ]
      : [
          {
            label: copy.names.askRemove(short(record.name)),
            onSelect: () => openAsk(copy.askAdmin.request.dnsRm(netName, label(record.name))),
          },
        ];
  }

  // Members: the request to send an admin (§7.10).
  let askOpen = $state(false);
  let askRequest = $state('');
  function openAsk(request: string): void {
    askRequest = request;
    askOpen = true;
  }

  const id = $props.id();
  const addId = `${id}-add`;
  const formId = `${id}-form`;
  const reasonId = `${id}-reason`;
  const moreId = `${id}-more`;
  const memberId = `${id}-member`;

  const showAdd = $derived(
    network !== null && ready && !adding && (names.length > 0 || loadFailure !== null),
  );
  const addLabel = $derived(admin ? copy.names.add : copy.names.askAdd);
  const submitLabel = $derived(
    !admin ? copy.detail.askAdmin : existing ? copy.names.replace : copy.names.add,
  );
</script>

<section class="screen">
  {#if showAdd || (frozen && admin)}
    <div class="toolbar">
      {#if frozen && admin}<p class="reason" id={reasonId}>{copy.conn.staleActions}</p>{/if}
      {#if showAdd}
        <Button
          id={addId}
          disabled={frozen && admin}
          aria-describedby={frozen && admin ? reasonId : undefined}
          onclick={open}
        >
          {addLabel}
        </Button>
      {/if}
    </div>
  {/if}

  {#if network && ready}
    {#if names.length > 0 || adding}
      <div class="table" role="table" aria-label={copy.nav.names}>
        <div class="row head t-micro" role="row">
          <span role="columnheader">{copy.names.name}</span>
          <span role="columnheader">{copy.names.value}</span>
          <span role="columnheader"><span class="sr-only">{copy.names.setByColumn}</span></span>
          <span role="columnheader"><span class="sr-only">{copy.copyField.copy}</span></span>
          <span role="columnheader"><span class="sr-only">{copy.device.columns.actions}</span></span>
        </div>

        {#if adding}
          <form class="add-form" id={formId} novalidate onsubmit={submit} bind:this={formRef}>
            <div class="row add" role="row" aria-label={copy.names.add}>
            <div class="c-name" role="cell">
              <Field
                label={copy.names.name}
                hideLabel
                error={nameError}
                helper={nameHelp}
                hint={spacesHint ? copy.createNetwork.spaces : null}
              >
                {#snippet children(control)}
                  <Input
                    {...control}
                    kind="name"
                    class="name-box"
                    suffix={copy.names.suffix(netName)}
                    bind:value={nameText}
                    bind:ref={nameRef}
                    readonly={pending}
                    oninput={nameInput}
                    onblur={nameBlur}
                  />
                {/snippet}
              </Field>
            </div>
            <div class="c-value" role="cell">
              <Field
                label={copy.names.value}
                hideLabel
                error={valueError}
                helper={text ? copy.names.textHelp : copy.names.valueHelp(netName)}
              >
                {#snippet children(control)}
                  <Input
                    {...control}
                    kind={text ? 'text' : 'mono'}
                    bind:value={valueText}
                    bind:ref={valueRef}
                    readonly={pending}
                    autocapitalize="none"
                    autocorrect="off"
                    spellcheck={false}
                    enterkeyhint="done"
                    oninput={valueInput}
                    onblur={valueBlur}
                  />
                {/snippet}
              </Field>
              <div class="more-opts">
                <Button
                  variant="ghost"
                  aria-expanded={more}
                  aria-controls={more ? moreId : undefined}
                  onclick={() => (more = !more)}
                >
                  {copy.names.more}
                </Button>
                {#if more}
                  <div class="txt" id={moreId}>
                    <Switch label={copy.names.txt} bind:checked={text} disabled={pending} />
                  </div>
                {/if}
              </div>
            </div>
            <div class="c-actions" role="cell">
              <Button variant="ghost" disabled={pending} onclick={() => void close()}>
                {copy.action.cancel}
              </Button>
              <Button
                variant="primary"
                type="submit"
                {pending}
                disabled={frozen && admin}
                aria-describedby={frozen && admin ? reasonId : !admin ? memberId : undefined}
              >
                {submitLabel}
              </Button>
            </div>
            {#if formError || pending || !admin}
              <p class="c-note" class:failed={formError !== null}>
                {#if formError}{formError}{:else if pending}{copy.action.signing}{:else}<span
                    id={memberId}>{copy.error.permissionDenied}</span
                  >{/if}
              </p>
            {/if}
            </div>
          </form>
        {/if}

        {#each names as record (record.name)}
          {@const to = target(record)}
          {@const op = ops[record.name]}
          {@const busy = op?.state === 'pending'}
          <div
            class="row"
            class:tint={netId !== null && live.changed.has(tintKey.record(netId, record.name))}
            role="row"
            aria-busy={busy || undefined}
            data-name={record.name}
            animate:flip={{ duration: rowMs }}
            transition:slide={{ duration: rowMs }}
          >
            <span class="name t-mono" role="cell">{short(record.name)}</span>
            <span class="target" role="cell">
              {#if record.type !== 'TXT'}<span class="arrow">{copy.names.arrow}</span>{/if}<span
                class:t-mono={to.mono}>{to.text}</span
              >
              {#if to.address}<span class="address t-mono">{DOT}{to.address}</span>{/if}
            </span>
            <span class="by" role="cell">
              {#if op}
                <span>{progress(op, short(record.name))}</span>
              {:else if record.set_by}
                <span>{copy.names.setBy(record.set_by)}</span>
                <Icon name="seal" />
              {/if}
            </span>
            <span class="copy" role="cell">
              <button
                type="button"
                class="copy-btn"
                disabled={busy}
                aria-label={copy.names.copyName(record.name)}
                onclick={() => copyName(record.name)}
              >
                {#if copied === record.name}
                  <Icon name="check" />
                  <span>{copy.copyField.copied}</span>
                {:else}
                  <span>{copy.copyField.copy}</span>
                {/if}
              </button>
            </span>
            <span class="more" role="cell">
              <DropdownMenu items={items(record)}>
                {#snippet trigger(props)}
                  <IconButton
                    icon="ellipsis"
                    label={copy.names.moreFor(short(record.name))}
                    disabled={busy}
                    triggerProps={props}
                  />
                {/snippet}
              </DropdownMenu>
            </span>
          </div>
        {/each}
      </div>
      <span class="sr-only" aria-live="polite">{copied ? copy.copyField.copied : ''}</span>
    {:else if loadFailure === null}
      <div class="empty">
        <p>{copy.names.empty(netName)}</p>
        <div>
          <Button
            id={addId}
            variant="primary"
            disabled={frozen && admin}
            aria-describedby={frozen && admin ? reasonId : undefined}
            onclick={open}
          >
            {addLabel}
          </Button>
        </div>
      </div>
    {/if}

    {#if loadFailure}
      <div class="load-failed">
        <p class="failed">
          {loadFailure.code === 'not_implemented'
            ? copy.error.notImplemented
            : copy.error.internal(loadFailure.message)}
        </p>
        <Button variant="secondary" onclick={retry}>{copy.names.tryAgain}</Button>
      </div>
    {/if}
  {/if}
</section>

<AskAdminSheet bind:open={askOpen} {net} request={askRequest} />

<style>
  .screen {
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--content-pad);
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-4);
    min-height: var(--control);
  }
  .toolbar .reason {
    flex: 1;
    min-width: 0;
  }
  .table {
    /* Wide enough for `Copied` with its check, so the row never shifts. */
    --copy-col: calc(var(--s-8) + var(--s-5));
    display: grid;
  }
  .row {
    display: grid;
    grid-template-columns:
      minmax(0, 1fr) minmax(0, 1.4fr) minmax(0, 0.8fr) var(--copy-col)
      var(--control);
    align-items: center;
    gap: var(--s-4);
    min-height: var(--row);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .row:not(.head):not(.add):hover {
    background: var(--sunken);
  }
  .add-form {
    display: contents;
  }
  .add {
    align-items: start;
    row-gap: var(--s-2);
    padding-top: var(--s-2);
    padding-bottom: var(--s-2);
    background: var(--surface);
  }
  .c-name,
  .c-value {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .add :global(.name-box input) {
    font-family: var(--ff-mono);
  }
  .more-opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
  .txt {
    display: inline-flex;
    max-width: var(--measure);
  }
  .c-actions {
    grid-column: 3 / -1;
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--s-2);
  }
  .c-note {
    grid-column: 1 / -1;
    font: var(--t-small);
    color: var(--ink-3);
  }
  .tint {
    animation: tint var(--d-tint) var(--e-linear);
  }
  @keyframes tint {
    from {
      background-color: var(--changed);
    }
  }
  .head {
    min-height: var(--control);
    color: var(--ink-3);
  }
  .name,
  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .target {
    color: var(--ink-2);
  }
  .arrow,
  .address {
    color: var(--ink-3);
  }
  .by {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    font: var(--t-small);
    color: var(--ink-3);
  }
  .copy-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--s-1);
    width: 100%;
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    font-weight: 400;
    color: var(--ink-2);
    cursor: pointer;
    transition:
      background-color var(--d-hover) var(--e-linear),
      color var(--d-hover) var(--e-linear);
  }
  .copy-btn:not(:disabled):hover {
    background: var(--surface);
    color: var(--ink-1);
  }
  .copy-btn:disabled {
    color: var(--ink-3);
    cursor: default;
  }
  .more {
    display: flex;
    justify-content: flex-end;
  }
  .empty {
    display: grid;
    gap: var(--s-4);
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .load-failed {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
  .failed {
    color: var(--danger);
  }
  .c-note.failed {
    color: var(--danger);
  }
  .reason {
    color: var(--ink-3);
  }
  @media (pointer: coarse) {
    .copy-btn {
      height: var(--target-touch);
    }
  }
  @media (max-width: 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
    .head {
      display: none;
    }
    .row {
      grid-template-columns: minmax(0, 1fr) var(--copy-col) var(--control);
      grid-template-areas:
        'name copy more'
        'target by by';
      gap: 0 var(--s-3);
      min-height: var(--row-mobile);
      padding: var(--s-2) 0;
    }
    .add {
      grid-template-columns: minmax(0, 1fr);
      grid-template-areas: none;
      gap: var(--s-3);
      padding: var(--s-3);
    }
    .add > * {
      grid-column: 1 / -1;
    }
    /* No column headers on a phone: the labels show above the fields again. */
    .add :global(.hidden-label) {
      position: static;
      width: auto;
      height: auto;
      margin: 0;
      overflow: visible;
      clip-path: none;
      white-space: normal;
    }
    .name {
      grid-area: name;
    }
    .target {
      grid-area: target;
      font: var(--t-small);
    }
    .by {
      grid-area: by;
      justify-content: flex-end;
    }
    .copy {
      grid-area: copy;
    }
    .more {
      grid-area: more;
    }
  }
</style>
