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

  const themeLabel = $derived(
    theme === "light" ? ko.themeLight : theme === "contrast" ? ko.themeContrast : ko.themeDark,
  );
</script>

<header class="bar">
  <h1>{ko.appTitle}</h1>

  <div class="group" role="status" aria-live="polite">
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
