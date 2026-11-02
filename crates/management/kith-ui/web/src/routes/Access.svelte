<!--
@component
Access (M1.1): the network's policy. First every rule as a plain sentence (`Phones can reach
nas on port 22.`), then, for admins, the editor: `Rules` adds and removes rules one at a
time and shows what will change before anything is signed; `TOML` is the policy exactly as
kith reads it, and a parse error shows inline at its line and column. Both save the whole
policy through `POST /api/networks/:net/policy`. Members read the sentences and get `Ask an
admin…` where the editor would be.
Props:
  - net: string, the network's id or name (as in the URL).
  - query?: URLSearchParams. `?new=1` (the palette's `Add access rule`) focuses the new rule.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { announce } from '../lib/announce';
  import { errorText } from '../lib/api';
  import { copy } from '../lib/copy';
  import AskAdminSheet from '../lib/kith/AskAdminSheet.svelte';
  import { findNetwork, membershipOf } from '../lib/kith/network';
  import {
    errorPosition,
    offsetOf,
    parsePorts,
    policyToml,
    readRules,
    ruleSentence,
    tagsIn,
    type NameOf,
  } from '../lib/kith/policy';
  import { actions, live } from '../lib/live/store.svelte';
  import type {
    ActionHandle,
    PolicyProtocol,
    PolicyRule,
    PolicySelector,
    PolicyView,
  } from '../lib/types';
  import type { Outcome } from '../lib/types.gen';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Combobox from '../lib/ui/Combobox.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { setQuery } from '../router';

  let { net, query }: { net: string; query?: URLSearchParams } = $props();

  const network = $derived(findNetwork(live.status, net));
  const id = $derived(network?.id ?? null);
  const self = $derived(id ? membershipOf(live.status, id) : null);
  const admin = $derived(self?.role === 'admin');
  const frozen = $derived(live.conn === 'stale' || live.conn === 'daemon_down');
  const policy = $derived<PolicyView | null>(id ? (live.policies[id] ?? null) : null);

  let loading = $state<ActionHandle<PolicyView> | null>(null);
  $effect(() => {
    const net = id;
    if (net) loading = untrack(() => actions.loadPolicy(net));
  });

  const devices = $derived.by(() => {
    if (!id) return [];
    const list = Object.values(live.peers[id] ?? {}).map((p) => ({ node: p.node, name: p.name }));
    const me = live.status?.node;
    if (self && me) list.push({ node: me, name: self.name });
    return list.sort((a, b) => a.name.localeCompare(b.name));
  });
  const nameOf: NameOf = (node) => devices.find((d) => d.node === node)?.name ?? null;

  const saved = $derived(policy ? readRules(policy.rules) : null);
  const unreadable = $derived(policy !== null && saved === null);

  // The draft: saved rules marked for removal, and rules to add. Nothing is signed until Save.
  let removed = $state<number[]>([]);
  let added = $state<PolicyRule[]>([]);
  const draft = $derived([
    ...(saved ?? []).filter((_, i) => !removed.includes(i)),
    ...added,
  ]);
  const changes = $derived(removed.length + added.length);
  // A newer policy from elsewhere replaces a draft built on the old one.
  $effect(() => {
    void policy;
    untrack(() => {
      removed = [];
      added = [];
    });
  });

  let saving = $state<ActionHandle<Outcome> | null>(null);
  const savingNow = $derived(saving?.state === 'pending');

  async function saveRules(): Promise<void> {
    if (!id || frozen || savingNow || changes === 0) return;
    const handle = actions.setPolicy(id, policyToml(draft));
    saving = handle;
    await handle.done;
    if (handle.state === 'failed' && handle.error) {
      announce(errorText(handle.error));
      return;
    }
    if (handle.state === 'conflict') return;
    removed = [];
    added = [];
    toast.done(copy.access.saved);
  }

  // The new rule.
  type Pick = string;
  const TAG_NEW = 'tag-new';
  const tags = $derived(tagsIn([...(saved ?? []), ...added]));
  const pickOptions = $derived([
    { value: 'any', label: copy.access.everyDevice },
    ...devices.map((d) => ({ value: `device:${d.node}`, label: d.name })),
    ...tags.map((t) => ({ value: `tag:${t}`, label: t })),
    { value: TAG_NEW, label: copy.access.tagOption },
  ]);
  let from = $state<Pick>('any');
  let to = $state<Pick>('any');
  let fromTag = $state('');
  let toTag = $state('');
  let protocol = $state<PolicyProtocol>('tcp');
  let ports = $state('');
  let ruleError = $state<string | null>(null);
  let portsError = $state<string | null>(null);
  let formRef = $state<HTMLFormElement | null>(null);

  const protocolOptions = (['tcp', 'udp', 'icmp', 'any'] as const).map((value) => ({
    value,
    label: copy.access.protocols[value],
  }));

  function selector(pick: Pick, tag: string): PolicySelector | null {
    if (pick === 'any') return { kind: 'any' };
    if (pick.startsWith('device:')) return { kind: 'device', node: pick.slice('device:'.length) };
    if (pick.startsWith('tag:')) return { kind: 'tag', tag: pick.slice('tag:'.length) };
    const t = tag.trim().toLowerCase();
    return /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(t) ? { kind: 'tag', tag: t } : null;
  }

  /** The rule the form describes, or the reason it describes none. */
  const building = $derived.by((): PolicyRule | string => {
    const source = selector(from, fromTag);
    const target = selector(to, toTag);
    if (!source || !target) return copy.access.tagInvalid;
    const parsed = parsePorts(ports);
    if (parsed === 'invalid') return copy.access.portsInvalid;
    if (protocol === 'icmp' && parsed !== null) return copy.access.portsPing;
    return {
      sources: [source],
      destinations: [{ selector: target, protocol, ports: parsed }],
    };
  });

  function same(a: PolicyRule, b: PolicyRule): boolean {
    return policyToml([a]) === policyToml([b]);
  }

  function addRule(event: SubmitEvent): void {
    event.preventDefault();
    const rule = building;
    if (typeof rule === 'string') {
      if (rule === copy.access.portsInvalid || rule === copy.access.portsPing) {
        portsError = rule;
        ruleError = null;
      } else {
        ruleError = rule;
        portsError = null;
      }
      announce(rule);
      return;
    }
    if (draft.some((r) => same(r, rule))) {
      ruleError = copy.access.duplicate;
      announce(ruleError);
      return;
    }
    ruleError = null;
    portsError = null;
    added = [...added, rule];
    ports = '';
    announce(copy.access.preview(ruleSentence(rule, nameOf)));
  }

  // `?new=1` from the palette: focus the new rule's first field, then drop the parameter.
  $effect(() => {
    if (query?.get('new') !== '1' || !admin || !formRef) return;
    untrack(() => {
      setQuery({ new: null }, { replace: true });
      void tick().then(() => formRef?.querySelector('input')?.focus());
    });
  });

  // TOML.
  let view = $state<'rules' | 'toml'>('rules');
  let text = $state('');
  let textRef = $state<HTMLTextAreaElement | null>(null);
  let tomlSaving = $state<ActionHandle<Outcome> | null>(null);
  let tomlError = $state<string | null>(null);
  $effect(() => {
    const toml = policy?.toml ?? '';
    untrack(() => {
      text = toml;
      tomlError = null;
    });
  });
  $effect(() => {
    if (unreadable) view = 'toml';
  });

  async function saveToml(): Promise<void> {
    if (!id || frozen || tomlSaving?.state === 'pending') return;
    const handle = actions.setPolicy(id, text);
    tomlSaving = handle;
    tomlError = null;
    await handle.done;
    if (handle.state === 'failed' && handle.error) {
      const failure = handle.error;
      const at = failure.code === 'invalid_argument' ? errorPosition(failure.message) : null;
      const message = failure.message.split('\n').at(-1)?.trim() || failure.message;
      tomlError = at ? copy.access.tomlError(at.line, at.column, message) : errorText(failure);
      announce(tomlError);
      if (at && textRef) {
        const offset = offsetOf(text, at.line, at.column);
        textRef.focus();
        textRef.setSelectionRange(offset, offset);
      }
      return;
    }
    if (handle.state !== 'conflict') toast.done(copy.access.saved);
  }

  let askOpen = $state(false);
  const tabs = [
    { value: 'rules', label: copy.access.views.rules },
    { value: 'toml', label: copy.access.views.toml },
  ];
  const headingId = $props.id();
  const reasonId = `${headingId}-reason`;
</script>

<section class="screen">
  {#if network}
    <section class="group" aria-labelledby={headingId}>
      <h2 class="t-heading" id={headingId}>{copy.access.heading}</h2>
      <p class="quiet">{copy.access.intro(network.name)}</p>
      {#if loading?.state === 'failed' && loading.error}
        <p class="error">{errorText(loading.error)}</p>
      {:else if unreadable}
        <p class="quiet">{copy.access.unreadable}</p>
      {:else if saved}
        {#if draft.length === 0 && (saved.length === 0 || changes > 0)}
          <p>{copy.access.none(network.name)}</p>
        {/if}
        <ul class="rules">
          {#each saved as rule, i (i)}
            {@const sentence = ruleSentence(rule, nameOf)}
            {@const going = removed.includes(i)}
            <li class="rule" class:going>
              <span class="sentence">{sentence}</span>
              {#if going}<Badge>{copy.access.goes}</Badge>{/if}
              {#if admin}
                {#if going}
                  <Button
                    variant="ghost"
                    aria-label={copy.access.keepLabel(sentence)}
                    onclick={() => (removed = removed.filter((r) => r !== i))}
                  >
                    {copy.access.keep}
                  </Button>
                {:else}
                  <Button
                    variant="ghost"
                    aria-label={copy.access.removeLabel(sentence)}
                    onclick={() => (removed = [...removed, i])}
                  >
                    {copy.access.remove}
                  </Button>
                {/if}
              {/if}
            </li>
          {/each}
          {#each added as rule, i (i)}
            {@const sentence = ruleSentence(rule, nameOf)}
            <li class="rule">
              <span class="sentence">{sentence}</span>
              <Badge>{copy.access.isNew}</Badge>
              <Button
                variant="ghost"
                aria-label={copy.access.removeLabel(sentence)}
                onclick={() => (added = added.filter((_, j) => j !== i))}
              >
                {copy.access.remove}
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if !admin}
      <section class="group">
        <p class="quiet">{copy.access.memberNote}</p>
        <div><Button onclick={() => (askOpen = true)}>{copy.access.ask}</Button></div>
      </section>
      <AskAdminSheet bind:open={askOpen} {net} request={copy.askAdmin.request.policy(network.name)} />
    {:else if policy}
      <Tabs bind:value={view} {tabs} label={copy.access.viewsLabel}>
        {#snippet panel(tab)}
          {#if tab === 'rules' && saved}
            <div class="editor">
              {#if changes > 0}
                <div class="pending" role="status">
                  <p>{copy.access.unsaved(changes)}</p>
                  {#if removed.length > 0}<p class="quiet">{copy.access.losesAccess}</p>{/if}
                  {#if saving?.state === 'failed' && saving.error}
                    <p class="error">{errorText(saving.error)}</p>
                  {/if}
                  <div class="buttons">
                    <Button
                      variant="ghost"
                      onclick={() => {
                        removed = [];
                        added = [];
                      }}
                    >
                      {copy.access.discard}
                    </Button>
                    <Button
                      variant="primary"
                      pending={savingNow}
                      pendingLabel={copy.action.signing}
                      disabled={frozen}
                      aria-describedby={frozen ? reasonId : undefined}
                      onclick={() => void saveRules()}
                    >
                      {copy.access.save}
                    </Button>
                  </div>
                </div>
              {/if}

              <form class="add" novalidate bind:this={formRef} onsubmit={addRule}>
                <h3 class="label">{copy.access.addRule}</h3>
                <div class="pickers">
                  <div class="pick">
                    <span class="field-label" aria-hidden="true">{copy.access.from}</span>
                    <Combobox
                      value={from}
                      options={pickOptions}
                      label={copy.access.from}
                      empty={copy.access.noDevices}
                      onValueChange={(v) => (from = v)}
                    />
                    {#if from === TAG_NEW}
                      <Field label={copy.access.tag}>
                        {#snippet children(control)}
                          <Input {...control} kind="name" bind:value={fromTag} />
                        {/snippet}
                      </Field>
                    {/if}
                  </div>
                  <div class="pick">
                    <span class="field-label" aria-hidden="true">{copy.access.to}</span>
                    <Combobox
                      value={to}
                      options={pickOptions}
                      label={copy.access.to}
                      empty={copy.access.noDevices}
                      onValueChange={(v) => (to = v)}
                    />
                    {#if to === TAG_NEW}
                      <Field label={copy.access.tag}>
                        {#snippet children(control)}
                          <Input {...control} kind="name" bind:value={toTag} />
                        {/snippet}
                      </Field>
                    {/if}
                  </div>
                </div>
                <div class="pick">
                  <span class="field-label" aria-hidden="true">{copy.access.protocol}</span>
                  <Segmented
                    value={protocol}
                    options={protocolOptions}
                    label={copy.access.protocol}
                    onValueChange={(v) => (protocol = v as PolicyProtocol)}
                  />
                </div>
                {#if protocol !== 'icmp'}
                  <Field label={copy.access.ports} helper={copy.access.portsHelp} error={portsError}>
                    {#snippet children(control)}
                      <Input
                        {...control}
                        kind="mono"
                        inputmode="numeric"
                        bind:value={ports}
                        oninput={() => (portsError = null)}
                      />
                    {/snippet}
                  </Field>
                {/if}
                {#if typeof building !== 'string'}
                  <p class="preview">{copy.access.preview(ruleSentence(building, nameOf))}</p>
                {/if}
                {#if ruleError}<p class="error">{ruleError}</p>{/if}
                <div><Button type="submit">{copy.access.addRule}</Button></div>
              </form>
            </div>
          {:else}
            <div class="editor">
              <label class="field-label" for="{headingId}-toml">{copy.access.toml}</label>
              <p class="quiet" id="{headingId}-toml-help">{copy.access.tomlHelp}</p>
              <textarea
                id="{headingId}-toml"
                class="toml"
                bind:this={textRef}
                bind:value={text}
                spellcheck="false"
                autocapitalize="none"
                autocomplete="off"
                rows="16"
                aria-invalid={tomlError ? 'true' : undefined}
                aria-describedby="{headingId}-toml-help{tomlError ? ` ${headingId}-toml-error` : ''}"
                oninput={() => (tomlError = null)}
              ></textarea>
              {#if tomlError}
                <p class="error mono" id="{headingId}-toml-error">{tomlError}</p>
              {/if}
              <div class="buttons">
                {#if text !== policy.toml}
                  <Button variant="ghost" onclick={() => (text = policy?.toml ?? '')}>
                    {copy.access.tomlRevert}
                  </Button>
                {/if}
                <Button
                  variant="primary"
                  pending={tomlSaving?.state === 'pending'}
                  pendingLabel={copy.action.signing}
                  disabled={frozen}
                  aria-describedby={frozen ? reasonId : undefined}
                  onclick={() => void saveToml()}
                >
                  {copy.access.save}
                </Button>
              </div>
            </div>
          {/if}
        {/snippet}
      </Tabs>
      {#if frozen}<p class="quiet" id={reasonId}>{copy.conn.staleActions}</p>{/if}
    {/if}
  {/if}
</section>

<style>
  .screen {
    display: grid;
    gap: var(--s-7);
    max-width: var(--content-max);
    margin: 0;
    padding: var(--s-6) var(--content-pad);
  }
  .group,
  .editor,
  .add {
    display: grid;
    gap: var(--s-3);
    max-width: calc(var(--measure) + var(--s-8));
  }
  .rules {
    display: grid;
  }
  .rule {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row);
    border-bottom: 1px solid var(--line);
  }
  .sentence {
    flex: 1;
    min-width: 0;
  }
  .rule.going .sentence {
    color: var(--ink-3);
    text-decoration: line-through;
  }
  .pending {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-4);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-2);
    background: var(--surface);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--s-3);
  }
  .pickers {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: var(--s-4);
  }
  .pick {
    display: grid;
    justify-items: start;
    gap: var(--s-1);
  }
  .pick :global(.box) {
    width: 100%;
  }
  .field-label {
    font-weight: 600;
  }
  .label {
    padding-top: var(--s-3);
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
  }
  .preview {
    color: var(--ink-2);
  }
  .toml {
    width: 100%;
    min-height: calc(var(--row) * 8);
    padding: var(--s-3);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-1);
    font: var(--t-mono);
    resize: vertical;
  }
  .toml:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .toml[aria-invalid='true'] {
    border-color: var(--danger);
  }
  .quiet {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
  .error {
    font: var(--t-small);
    color: var(--danger);
  }
  .error.mono {
    font: var(--t-mono);
    white-space: pre-wrap;
  }

  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
  }
</style>
