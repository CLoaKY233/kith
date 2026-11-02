<!--
@component
This device (`/this`, spec §6.8): its name, key fingerprints with Copy, its role in each
network, the versions of kith-ui and kith, and the daemon log (mono, virtualised, newest
first). `Copy redacted log` copies the log oldest first with IPs shortened to `81.2.x.x`
and keys to their first characters.
Props:
  none
-->
<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity';
  import { copy, copyScreens, DOT } from '../lib/copy';
  import { logLevel, logLine } from '../lib/kith/activity';
  import EventRow from '../lib/kith/EventRow.svelte';
  import VirtualList from '../lib/kith/VirtualList.svelte';
  import { live } from '../lib/live/store.svelte';
  import { COPIED_MS } from '../lib/live/timing';
  import { readPx } from '../lib/tokens';
  import type { Entry } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import RoleChip from '../lib/ui/RoleChip.svelte';

  const status = $derived(live.status);
  const name = $derived(status?.memberships[0]?.name ?? status?.host.name ?? '');
  const rows = $derived(
    (status?.memberships ?? []).map((m) => ({
      membership: m,
      network: status?.networks.find((n) => n.id === m.network)?.name ?? m.network,
    })),
  );
  /** One fingerprint per key; a network name only when keys differ between networks. */
  const keys = $derived(
    [...new Set(rows.map((r) => r.membership.key_fpr))].map((fpr) => ({
      fpr,
      networks: rows.filter((r) => r.membership.key_fpr === fpr).map((r) => r.network),
    })),
  );

  const lines = $derived(live.journal.filter((e) => logLevel(e) !== null).reverse());
  const rowPx = readPx('row');
  const open = new SvelteSet<number>();

  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copyLog(): Promise<void> {
    const text = live.journal
      .map(logLine)
      .filter((line): line is string => line !== null)
      .join('\n');
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      failed = true;
      return;
    }
    failed = false;
    copied = true;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => (copied = false), COPIED_MS);
  }
</script>

<section class="screen">

  <dl class="facts">
    <div class="fact">
      <dt>{copy.thisDevice.name}</dt>
      <dd>{name}</dd>
    </div>
    <div class="fact">
      <dt>{copy.thisDevice.versions}</dt>
      <dd>
        <span>{copy.thisDevice.ui} {live.versions?.ui ?? copy.units.none}</span>
        <span>{copy.thisDevice.daemon} {live.versions?.daemon || copy.units.none}</span>
      </dd>
    </div>
  </dl>

  <section class="group" aria-labelledby="this-keys">
    <h2 class="t-heading" id="this-keys">{copy.thisDevice.keys}</h2>
    {#each keys as key (key.fpr)}
      <div class="key">
        <span class="label">
          {copy.thisDevice.fingerprint}{keys.length > 1 ? `${DOT}${key.networks.join(', ')}` : ''}
        </span>
        <CopyField value={key.fpr} label={copy.thisDevice.fingerprint} />
      </div>
    {/each}
  </section>

  <section class="group" aria-labelledby="this-roles">
    <h2 class="t-heading" id="this-roles">{copy.thisDevice.roles}</h2>
    {#if rows.length === 0}
      <p class="quiet">{copyScreens.thisDevice.noNetworks}</p>
    {:else}
      <table class="roles">
        <thead>
          <tr>
            <th scope="col">{copyScreens.thisDevice.network}</th>
            <th scope="col">{copyScreens.thisDevice.address}</th>
            <th scope="col">{copyScreens.thisDevice.role}</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as row (row.membership.network)}
            <tr>
              <td>{row.network}</td>
              <td class="mono">{row.membership.address}</td>
              <td>
                <RoleChip
                  role={row.membership.role}
                  network={row.network}
                  keyFpr={row.membership.key_fpr}
                  madeAdminBy={row.membership.made_admin_by}
                  madeAdminAtMs={row.membership.made_admin_at_ms}
                  admins={row.membership.admins}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section class="group" aria-labelledby="this-log">
    <div class="log-head">
      <h2 class="t-heading" id="this-log">{copy.thisDevice.log}</h2>
      <Button variant="secondary" onclick={() => void copyLog()}>
        {copied ? copy.copyField.copied : copy.thisDevice.copyLog}
      </Button>
    </div>
    {#if failed}<p class="error">{copyScreens.thisDevice.copyFailed}</p>{/if}
    <span class="sr-only" aria-live="polite">{copied ? copy.copyField.copied : ''}</span>
    <div class="log" role="log" aria-live="off" aria-labelledby="this-log">
      {#if lines.length === 0}
        <p class="quiet">{copyScreens.thisDevice.logEmpty}</p>
      {:else}
        <VirtualList items={lines} key={(e: Entry) => String(e.seq)} estimate={rowPx}>
          {#snippet row(entry: Entry)}
            <EventRow
              {entry}
              link={false}
              expanded={open.has(entry.seq)}
              onToggle={(on) => (on ? open.add(entry.seq) : open.delete(entry.seq))}
            />
          {/snippet}
        </VirtualList>
      {/if}
    </div>
  </section>
</section>

<style>
  .screen {
    display: grid;
    gap: var(--s-7);
    max-width: var(--content-max);
    margin: 0;
    padding: var(--s-6) var(--content-pad);
  }
  .group {
    display: grid;
    gap: var(--s-3);
  }
  .facts {
    display: grid;
    gap: var(--s-2);
  }
  .fact {
    display: grid;
    grid-template-columns: calc(var(--s-8) * 2) minmax(0, 1fr);
    gap: var(--s-4);
    align-items: baseline;
  }
  dt,
  .label {
    color: var(--ink-3);
    font: var(--t-small);
  }
  dd {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
  }
  .key {
    display: grid;
    gap: var(--s-1);
    max-width: calc(var(--s-8) * 6);
  }
  .roles {
    width: 100%;
    max-width: calc(var(--measure) + var(--s-8));
    border-collapse: collapse;
  }
  th {
    padding: 0 var(--s-3) var(--s-2) 0;
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
    text-align: left;
  }
  td {
    height: var(--row);
    padding: 0 var(--s-3) 0 0;
  }
  .mono {
    font: var(--t-mono);
  }
  .log-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .quiet {
    padding: var(--s-3);
    color: var(--ink-3);
    font: var(--t-small);
  }
  .error {
    color: var(--danger);
    font: var(--t-small);
  }

  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
    .fact {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--s-1);
    }
  }
</style>
