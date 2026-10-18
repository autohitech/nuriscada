<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { formatMicros } from "../lib/format";
  import type { Store } from "../lib/store.svelte";
  import { ko } from "../lib/strings";
  import StateChip from "./StateChip.svelte";

  let {
    store,
    theme,
    onTheme,
  }: { store: Store; theme: string; onTheme: () => void } = $props();

  /** 장치가 많으면 한 줄 요약으로 줄이고 자세한 내용은 마우스를 올렸을 때 보여 준다. */
  const COMPACT_FROM = 5;
  const connectedCount = $derived(store.devices.filter((d) => d.connected).length);
  const worstJitter = $derived(
    store.devices.reduce<number | null>((m, d) => (d.jitter_p99_us === null ? m : Math.max(m ?? 0, d.jitter_p99_us)), null),
  );
  const details = $derived(
    store.devices
      .map((d) => `${d.name}: ${d.connected ? ko.connected : ko.badQuality}, ${ko.scans} ${d.scans}, ${ko.jitter} ${formatMicros(d.jitter_p99_us)}`)
      .join("\n"),
  );

  const themeLabel = $derived(
    theme === "light" ? ko.themeLight : theme === "contrast" ? ko.themeContrast : ko.themeDark,
  );
</script>

<header class="bar">
  <h1>{ko.appTitle}</h1>

  <div class="group" role="status" aria-live="polite">
    {#if store.archiveError}<span role="alert" title={store.archiveError}><StateChip severity="critical" label="알람 기록 오류" /></span>{/if}
    {#if store.connected}
      <StateChip severity="normal" label={ko.connected} />
    {:else}
      <StateChip severity="bad" label={ko.reconnecting} />
    {/if}
    {#if store.activeNames.length > 0}
      <StateChip severity="critical" label={`${ko.alarms} ${store.activeNames.length}`} />
    {:else}
      <StateChip severity="normal" label={`${ko.alarms} 0`} />
    {/if}
  </div>

  {#if store.devices.length >= COMPACT_FROM}
    <p class="devices summary" title={details}>
      <span class="dev">{ko.devices}</span>
      <StateChip
        severity={connectedCount === store.devices.length ? "normal" : "bad"}
        label={ko.devicesSummary(store.devices.length, connectedCount)}
      />
      <span class="muted">{ko.jitter} {formatMicros(worstJitter)}</span>
    </p>
  {:else}
  <ul class="devices" aria-label={ko.devices}>
    {#each store.devices as d (d.name)}
      <li title={d.last_error ?? ""}>
        <span class="dev">{d.name}</span>
        <StateChip severity={d.connected ? "normal" : "bad"} label={d.connected ? ko.connected : ko.badQuality} />
        <span class="muted">{ko.scans} {d.scans}</span>
        <span class="muted">{ko.jitter} {formatMicros(d.jitter_p99_us)}</span>
      </li>
    {/each}
  </ul>
  {/if}

  <button type="button" class="theme" onclick={onTheme} aria-label={`${ko.theme}: ${themeLabel}`}>
    {ko.theme}: {themeLabel}
  </button>
</header>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--gap) 20px;
    padding: 10px 16px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  h1 {
    margin: 0;
    font-size: 16px;
    letter-spacing: 0.02em;
  }
  .group,
  .devices li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .devices {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    list-style: none;
    margin: 0 auto 0 0;
    padding: 0;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block: 0;
  }
  .dev {
    font-weight: 600;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .theme {
    min-height: 32px;
    padding: 4px 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
  }
  .theme:hover {
    border-color: var(--accent);
  }
</style>
