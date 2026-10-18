<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { formatTime, formatValue, severityOfState } from "../lib/format";
  import type { Store } from "../lib/store.svelte";
  import { stateLabel, m } from "../lib/i18n.svelte";
  import type { TagSnapshot } from "../lib/types";
  import { visibleRange } from "../lib/window";
  import StateChip from "./StateChip.svelte";

  let { store }: { store: Store } = $props();

  const ROW = 36;
  let filter = $state("");
  let scrollTop = $state(0);
  let viewport = $state(400);

  // 검색어가 없으면 null(전체). 있으면 일치하는 태그의 위치 목록.
  const matches = $derived.by(() => {
    const f = filter.trim().toLowerCase();
    if (!f) return null;
    const out: number[] = [];
    store.names.forEach((n, i) => {
      if (n.toLowerCase().includes(f)) out.push(i);
    });
    return out;
  });
  const total = $derived(matches ? matches.length : store.names.length);
  const range = $derived(visibleRange(scrollTop, viewport, ROW, total));
  const positions = $derived(
    Array.from({ length: range.end - range.start }, (_, k) => range.start + k),
  );

  function tagAt(pos: number): TagSnapshot | undefined {
    return store.tags[matches ? (matches[pos] ?? -1) : pos];
  }
</script>

<section class="wrap" aria-label={m.tags}>
  <div class="tools">
    <label>
      <span class="sr-only">{m.filterLabel}</span>
      <input type="search" placeholder={m.filterPlaceholder} bind:value={filter} />
    </label>
    <span class="count">{total}</span>
  </div>

  <div class="table" role="table" aria-rowcount={total + 1} aria-label={m.tags}>
    <div class="head" role="rowgroup">
      <div class="row" role="row" aria-rowindex="1">
        <div role="columnheader">{m.colName}</div>
        <div role="columnheader" class="num">{m.colValue}</div>
        <div role="columnheader">{m.colState}</div>
        <div role="columnheader">{m.colChanged}</div>
        <div role="columnheader">{m.colDevice}</div>
      </div>
    </div>

    <div
      class="body"
      role="rowgroup"
      bind:clientHeight={viewport}
      onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
    >
      <div class="spacer" style:height="{total * ROW}px">
        {#each positions as pos (pos)}
          {@const t = tagAt(pos)}
          {#if t}
            <div class="row" role="row" aria-rowindex={pos + 2} style:top="{pos * ROW}px">
              <div role="cell" class="name">{t.name}</div>
              <div role="cell" class="num" class:stale={t.quality === "bad"}>
                {formatValue(t.value, m.on, m.off, m.noValue)}
                {#if t.kind === "analog"}<span class="unit">{t.unit_text}</span>{/if}
                {#if t.quality === "bad" && t.value !== null}<span class="hint">({m.lastValue})</span>{/if}
              </div>
              <div role="cell">
                {#if t.quality === "bad"}
                  <StateChip severity="bad" label={m.badQuality} />
                {:else if t.alarm_state}
                  <StateChip severity={severityOfState(t.alarm_state)} label={stateLabel(t.alarm_state)} />
                {/if}
              </div>
              <div role="cell" class="muted">{formatTime(t.changed_ms)}</div>
              <div role="cell" class="muted">{t.device}</div>
            </div>
          {/if}
        {/each}
      </div>
      {#if total === 0}
        <p class="empty">{filter.trim() ? m.noMatch : m.noData}</p>
      {/if}
    </div>
  </div>
</section>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    padding: 12px;
    gap: 8px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  input[type="search"] {
    min-width: 240px;
    padding: 6px 10px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    font: inherit;
  }
  .count {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .table {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .head {
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .spacer {
    position: relative;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(140px, 2fr) minmax(110px, 1.4fr) minmax(120px, 1.4fr) 90px minmax(70px, 1fr);
    align-items: center;
    gap: 8px;
    height: var(--row-h);
    padding: 0 12px;
  }
  .body .row {
    position: absolute;
    left: 0;
    right: 0;
    border-bottom: 1px solid var(--border);
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-family: var(--mono);
  }
  .unit,
  .hint,
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .stale {
    opacity: 0.7;
  }
  .empty {
    margin: 24px;
    color: var(--muted);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (max-width: 700px) {
    .row {
      grid-template-columns: minmax(100px, 2fr) minmax(90px, 1.4fr) minmax(100px, 1.4fr);
    }
    .row > :nth-child(n + 4) {
      display: none;
    }
  }
</style>
