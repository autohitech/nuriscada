<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { m } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { formatValue } from "../lib/format";
  import type { Store } from "../lib/store.svelte";
  import type {
    GraphicWidgetKind,
    GraphicWidgetSpec,
    ProjectSpec,
    ScreenSpec,
    TagSnapshot,
  } from "../lib/types";

  let {
    store,
    canEdit = false,
  }: {
    store: Store;
    canEdit?: boolean;
  } = $props();

  let screens = $state<ScreenSpec[]>([]);
  let activeIndex = $state(0);
  let editing = $state(false);
  let selectedWidgetId = $state<string | null>(null);
  let snapGrid = $state(true);
  let busy = $state(false);
  let statusMsg = $state("");
  let dragState = $state<{
    widgetId: string;
    startClientX: number;
    startClientY: number;
    origX: number;
    origY: number;
  } | null>(null);
  let svgEl = $state<SVGSVGElement | null>(null);

  const widgetKindLabel = (k: GraphicWidgetKind): string => m.widgetKind[k] ?? k;

  function buildAutoScreen(tags: TagSnapshot[]): ScreenSpec {
    const widgets: GraphicWidgetSpec[] = tags.slice(0, 12).map((t, idx) => {
      const col = idx % 4;
      const row = Math.floor(idx / 4);
      const kind: GraphicWidgetKind =
        t.kind === "analog"
          ? idx % 2 === 0
            ? "tank"
            : "gauge"
          : idx % 2 === 0
            ? "pump"
            : "lamp";
      return {
        id: `auto-${idx + 1}`,
        kind,
        label: t.name,
        tag: t.name,
        x: 24 + col * 190,
        y: 24 + row * 150,
        width: 170,
        height: 130,
        min: 0,
        max: 100,
      };
    });
    return {
      name: m.defaultScreenName,
      width: 800,
      height: Math.max(360, Math.ceil(Math.max(widgets.length, 1) / 4) * 150 + 40),
      widget: widgets,
    };
  }

  const displayScreens = $derived(
    screens.length > 0 ? screens : [buildAutoScreen(store.tags)]
  );
  const currentScreen = $derived(
    displayScreens[Math.min(activeIndex, Math.max(displayScreens.length - 1, 0))] ??
      buildAutoScreen(store.tags)
  );
  const selectedWidget = $derived(
    currentScreen.widget.find((w) => w.id === selectedWidgetId) ?? null
  );
  const editScreen = $derived(screens[activeIndex] ?? null);

  const tagByName = $derived.by(() => {
    const map = new Map<string, TagSnapshot>();
    for (const t of store.tags) {
      map.set(t.name, t);
    }
    return map;
  });

  async function loadScreens() {
    try {
      const res = await fetch("/api/v1/screens", { credentials: "same-origin" });
      if (!res.ok) return;
      const data = (await res.json()) as { screens?: ScreenSpec[] };
      if (Array.isArray(data.screens)) {
        screens = data.screens;
        if (activeIndex >= screens.length) activeIndex = 0;
      }
    } catch {
      // 오프라인 또는 재연결 중이면 이전 화면을 유지한다.
    }
  }

  onMount(() => {
    void loadScreens();
  });

  function ensureEditableScreens() {
    if (screens.length === 0) {
      screens = [structuredClone(buildAutoScreen(store.tags))];
      activeIndex = 0;
    }
  }

  function toggleEdit() {
    if (!editing) {
      ensureEditableScreens();
      selectedWidgetId = screens[activeIndex]?.widget[0]?.id ?? null;
      editing = true;
      statusMsg = "";
    } else {
      editing = false;
      selectedWidgetId = null;
    }
  }

  function addScreen() {
    ensureEditableScreens();
    const nextNum = screens.length + 1;
    screens.push({
      name: m.newScreenName(nextNum),
      width: 800,
      height: 420,
      widget: [],
    });
    activeIndex = screens.length - 1;
    selectedWidgetId = null;
  }

  function removeCurrentScreen() {
    if (screens.length <= 1) return;
    screens = screens.filter((_, i) => i !== activeIndex);
    activeIndex = Math.max(0, activeIndex - 1);
    selectedWidgetId = null;
  }

  function addWidget(kind: GraphicWidgetKind) {
    ensureEditableScreens();
    const scr = screens[activeIndex];
    if (!scr) return;
    const nextIdx = scr.widget.length + 1;
    let id = `w-${nextIdx}`;
    while (scr.widget.some((w) => w.id === id)) {
      id = `w-${Math.floor(Math.random() * 9000 + 1000)}`;
    }
    const defaultTag =
      kind === "label"
        ? null
        : (store.tags.find((t) =>
            kind === "tank" || kind === "gauge"
              ? t.kind === "analog"
              : kind === "pump" || kind === "valve" || kind === "lamp"
                ? t.kind === "digital"
                : true
          )?.name ??
          store.tags[0]?.name ??
          null);
    const col = (scr.widget.length % 4) * 180 + 20;
    const row = Math.floor(scr.widget.length / 4) * 140 + 20;
    const nw: GraphicWidgetSpec = {
      id,
      kind,
      label: defaultTag ?? widgetKindLabel(kind),
      tag: defaultTag,
      x: col,
      y: row,
      width: kind === "label" ? 200 : 160,
      height: kind === "tank" ? 150 : kind === "label" ? 50 : 120,
      min: 0,
      max: 100,
    };
    scr.widget.push(nw);
    selectedWidgetId = nw.id;
  }

  function autoLayoutFromTags() {
    ensureEditableScreens();
    const scr = screens[activeIndex];
    if (!scr) return;
    const generated = buildAutoScreen(store.tags);
    scr.widget = generated.widget;
    scr.height = generated.height;
    selectedWidgetId = scr.widget[0]?.id ?? null;
  }

  function removeSelectedWidget() {
    const scr = screens[activeIndex];
    if (!scr || !selectedWidgetId) return;
    scr.widget = scr.widget.filter((w) => w.id !== selectedWidgetId);
    selectedWidgetId = scr.widget[0]?.id ?? null;
  }

  async function saveScreens() {
    busy = true;
    statusMsg = "";
    try {
      const getRes = await fetch("/api/v1/project", { credentials: "same-origin" });
      const getBody = await getRes.json().catch(() => ({}));
      if (!getRes.ok) {
        throw new Error(getBody.error ?? m.projectLoadFailed(getRes.status));
      }
      const spec = (getBody as { project: ProjectSpec }).project;
      spec.screen = screens.map((s) => ({
        name: s.name.trim(),
        width: Number(s.width),
        height: Number(s.height),
        widget: s.widget.map((w) => ({
          id: w.id.trim(),
          kind: w.kind,
          label: w.label,
          tag: w.tag && w.tag.trim() ? w.tag.trim() : null,
          x: Number(w.x),
          y: Number(w.y),
          width: Number(w.width),
          height: Number(w.height),
          min: Number(w.min),
          max: Number(w.max),
        })),
      }));
      const postRes = await fetch("/api/v1/project", {
        method: "POST",
        credentials: "same-origin",
        headers: {
          "content-type": "application/json",
          "x-nuriscada-request": "1",
        },
        body: JSON.stringify(spec),
      });
      const postBody = await postRes.json().catch(() => ({}));
      if (!postRes.ok) {
        throw new Error(postBody.error ?? m.screenSaveFailed(postRes.status));
      }
      await loadScreens();
      statusMsg = m.screenSaved;
    } catch (e) {
      statusMsg = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function snap(v: number): number {
    return snapGrid ? Math.round(v / 10) * 10 : Math.round(v);
  }

  function onWidgetPointerDown(event: PointerEvent, w: GraphicWidgetSpec) {
    selectedWidgetId = w.id;
    if (!editing) return;
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    dragState = {
      widgetId: w.id,
      startClientX: event.clientX,
      startClientY: event.clientY,
      origX: w.x,
      origY: w.y,
    };
  }

  function onSvgPointerMove(event: PointerEvent) {
    if (!editing || !dragState || !svgEl) return;
    const scr = screens[activeIndex];
    if (!scr) return;
    const w = scr.widget.find((item) => item.id === dragState?.widgetId);
    if (!w) return;
    const rect = svgEl.getBoundingClientRect();
    const scaleX = rect.width > 0 ? scr.width / rect.width : 1;
    const scaleY = rect.height > 0 ? scr.height / rect.height : 1;
    const dx = (event.clientX - dragState.startClientX) * scaleX;
    const dy = (event.clientY - dragState.startClientY) * scaleY;
    w.x = Math.max(0, Math.min(scr.width - w.width, snap(dragState.origX + dx)));
    w.y = Math.max(0, Math.min(scr.height - w.height, snap(dragState.origY + dy)));
  }

  function onSvgPointerUp() {
    dragState = null;
  }

  function fillRatio(tag: TagSnapshot | undefined, w: GraphicWidgetSpec): number {
    if (!tag || typeof tag.value !== "number") return 0;
    const span = w.max - w.min;
    if (span <= 0) return 0;
    return Math.max(0, Math.min(1, (tag.value - w.min) / span));
  }

  function isActiveDigital(tag: TagSnapshot | undefined): boolean {
    if (!tag) return false;
    if (typeof tag.value === "boolean") return tag.value;
    if (typeof tag.value === "number") return tag.value > 0;
    return false;
  }

  function widgetStroke(tag: TagSnapshot | undefined, isSelected: boolean): string {
    if (isSelected) return "var(--accent, #4ea1ff)";
    if (!tag) return "var(--border)";
    if (tag.quality === "bad") return "var(--warn, #e5a100)";
    if (tag.alarm_state && tag.alarm_state !== "NORMAL") return "var(--danger, #ff5d5d)";
    return "var(--border)";
  }

  function statusBadgeText(tag: TagSnapshot | undefined): string {
    if (!tag) return "";
    if (tag.quality === "bad") return m.badCommShort;
    if (tag.alarm_state && tag.alarm_state !== "NORMAL") return m.alarmShort(tag.alarm_state);
    return m.normalShort;
  }
</script>

<section class="graphic-panel" aria-label={m.graphicPanel}>
  <header class="graphic-toolbar">
    <div class="tabs" role="tablist" aria-label={m.screenList}>
      {#each displayScreens as scr, idx (scr.name + "-" + idx)}
        <button
          type="button"
          role="tab"
          class:active={idx === activeIndex}
          aria-selected={idx === activeIndex}
          onclick={() => {
            activeIndex = idx;
            selectedWidgetId = displayScreens[idx]?.widget[0]?.id ?? null;
          }}
        >
          {scr.name}
        </button>
      {/each}
    </div>
    {#if canEdit}
      <div class="editor-actions">
        <button type="button" onclick={toggleEdit}>
          {editing ? m.closeEdit : m.editScreen}
        </button>
        {#if editing}
          <button type="button" onclick={addScreen}>{m.addScreenBtn}</button>
          <button type="button" disabled={screens.length <= 1} onclick={removeCurrentScreen}>
            {m.deleteScreen}
          </button>
          <button type="button" onclick={autoLayoutFromTags}>{m.autoLayout}</button>
          <button type="button" onclick={() => addWidget("tank")}>+ {m.widgetKind.tank}</button>
          <button type="button" onclick={() => addWidget("gauge")}>+ {m.widgetKind.gauge}</button>
          <button type="button" onclick={() => addWidget("pump")}>+ {m.widgetKind.pump}</button>
          <button type="button" onclick={() => addWidget("valve")}>+ {m.widgetKind.valve}</button>
          <button type="button" onclick={() => addWidget("lamp")}>+ {m.widgetKind.lamp}</button>
          <button type="button" onclick={() => addWidget("value")}>+ {m.widgetKind.value}</button>
          <button type="button" onclick={() => addWidget("label")}>+ {m.widgetKind.label}</button>
          <label class="snap-toggle">
            <input type="checkbox" bind:checked={snapGrid} /> {m.snap10}
          </label>
          <button type="button" class="primary-btn" disabled={busy} onclick={() => void saveScreens()}>
            {m.saveScreen}
          </button>
        {/if}
      </div>
    {/if}
  </header>

  {#if statusMsg}
    <p class="graphic-status" role="status">{statusMsg}</p>
  {/if}

  <div class="graphic-workspace" class:with-inspector={editing}>
    <div class="canvas-wrap">
      <svg
        bind:this={svgEl}
        class="graphic-svg"
        viewBox={`0 0 ${currentScreen.width} ${currentScreen.height}`}
        role="img"
        aria-label={currentScreen.name}
        onpointermove={onSvgPointerMove}
        onpointerup={onSvgPointerUp}
      >
        {#if editing && snapGrid}
          <defs>
            <pattern id="scada-grid-10" width="20" height="20" patternUnits="userSpaceOnUse">
              <path
                d="M 20 0 L 0 0 0 20"
                fill="none"
                stroke="var(--border)"
                stroke-width="0.5"
                stroke-opacity="0.5"
              />
            </pattern>
          </defs>
          <rect width={currentScreen.width} height={currentScreen.height} fill="url(#scada-grid-10)" />
        {/if}

        {#each currentScreen.widget as w (w.id)}
          {@const tag = w.tag ? tagByName.get(w.tag) : undefined}
          {@const isSelected = editing && selectedWidgetId === w.id}
          {@const ratio = fillRatio(tag, w)}
          {@const active = isActiveDigital(tag)}
          {@const stroke = widgetStroke(tag, isSelected)}
          <g
            class="scada-widget"
            role="button"
            tabindex="0"
            aria-label={w.label || w.tag || w.id}
            data-widget-id={w.id}
            data-widget-kind={w.kind}
            transform={`translate(${w.x}, ${w.y})`}
            onpointerdown={(e) => onWidgetPointerDown(e, w)}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") selectedWidgetId = w.id;
            }}
            style={editing ? "cursor: move;" : "cursor: pointer;"}
          >
            <rect
              x="0"
              y="0"
              width={w.width}
              height={w.height}
              rx="6"
              fill="var(--surface)"
              stroke={stroke}
              stroke-width={isSelected ? "2.5" : "1.5"}
              stroke-dasharray={tag?.quality === "bad" ? "5,3" : undefined}
            />

            <text x="10" y="20" fill="var(--text)" font-size="12" font-weight="600">
              {w.label || w.tag || w.id}
            </text>
            {#if tag}
              <text
                x={w.width - 10}
                y="20"
                text-anchor="end"
                fill={tag.quality === "bad"
                  ? "var(--warn, #e5a100)"
                  : tag.alarm_state && tag.alarm_state !== "NORMAL"
                    ? "var(--danger, #ff5d5d)"
                    : "var(--muted)"}
                font-size="10"
              >
                {statusBadgeText(tag)}
              </text>
            {/if}

            {#if w.kind === "tank"}
              {@const tankX = 18}
              {@const tankY = 30}
              {@const tankW = Math.max(40, w.width - 36)}
              {@const tankH = Math.max(36, w.height - 62)}
              {@const fillH = Math.round(tankH * ratio)}
              <rect
                x={tankX}
                y={tankY}
                width={tankW}
                height={tankH}
                rx="8"
                fill="var(--surface-2)"
                stroke="var(--border)"
              />
              <rect
                class="tank-fill"
                x={tankX + 2}
                y={tankY + tankH - fillH}
                width={tankW - 4}
                height={Math.max(0, fillH - 2)}
                rx="6"
                fill={tag?.alarm_state && tag.alarm_state !== "NORMAL"
                  ? "var(--danger, #ff5d5d)"
                  : "var(--accent, #3b82f6)"}
                fill-opacity="0.75"
              />
              <text
                x={w.width / 2}
                y={w.height - 10}
                text-anchor="middle"
                fill="var(--text)"
                font-size="13"
                font-weight="600"
              >
                {tag ? `${formatValue(tag.value)} ${tag.unit_text}`.trim() : "--"}
              </text>
            {:else if w.kind === "gauge"}
              {@const barX = 14}
              {@const barY = Math.max(36, w.height / 2 - 10)}
              {@const barW = Math.max(40, w.width - 28)}
              <rect
                x={barX}
                y={barY}
                width={barW}
                height="16"
                rx="8"
                fill="var(--surface-2)"
                stroke="var(--border)"
              />
              <rect
                class="gauge-fill"
                x={barX}
                y={barY}
                width={Math.round(barW * ratio)}
                height="16"
                rx="8"
                fill={tag?.alarm_state && tag.alarm_state !== "NORMAL"
                  ? "var(--danger, #ff5d5d)"
                  : "var(--ok, #22c55e)"}
              />
              <text
                x={w.width / 2}
                y={w.height - 14}
                text-anchor="middle"
                fill="var(--text)"
                font-size="14"
                font-weight="600"
              >
                {tag ? `${formatValue(tag.value)} ${tag.unit_text}`.trim() : "--"}
              </text>
            {:else if w.kind === "pump" || w.kind === "valve"}
              {@const cx = w.width / 2}
              {@const cy = w.height / 2 + 2}
              {#if w.kind === "pump"}
                <circle
                  cx={cx}
                  cy={cy}
                  r="22"
                  fill={active ? "var(--ok, #22c55e)" : "var(--surface-2)"}
                  fill-opacity={active ? "0.28" : "1"}
                  stroke={active ? "var(--ok, #22c55e)" : "var(--muted)"}
                  stroke-width="2"
                />
                <polygon
                  points={`${cx - 8},${cy - 10} ${cx + 12},${cy} ${cx - 8},${cy + 10}`}
                  fill={active ? "var(--ok, #22c55e)" : "var(--muted)"}
                />
              {:else}
                <polygon
                  points={`${cx - 22},${cy - 14} ${cx},${cy} ${cx - 22},${cy + 14} ${cx + 22},${cy - 14} ${cx},${cy} ${cx + 22},${cy + 14}`}
                  fill={active ? "var(--ok, #22c55e)" : "var(--surface-2)"}
                  stroke={active ? "var(--ok, #22c55e)" : "var(--muted)"}
                  stroke-width="2"
                />
              {/if}
              <text
                x={cx}
                y={w.height - 10}
                text-anchor="middle"
                fill="var(--text)"
                font-size="12"
                font-weight="600"
              >
                {active ? m.pumpOn : m.pumpOff}
              </text>
            {:else if w.kind === "lamp"}
              {@const cx = w.width / 2}
              {@const cy = w.height / 2}
              <circle
                cx={cx}
                cy={cy}
                r="18"
                fill={active ? "var(--ok, #22c55e)" : "var(--surface-2)"}
                stroke={active ? "var(--ok, #22c55e)" : "var(--muted)"}
                stroke-width="2.5"
              />
              <text
                x={cx}
                y={w.height - 10}
                text-anchor="middle"
                fill="var(--text)"
                font-size="12"
                font-weight="600"
              >
                {active ? m.lampOn : m.lampOff}
              </text>
            {:else if w.kind === "value"}
              <text
                x={w.width / 2}
                y={w.height / 2 + 12}
                text-anchor="middle"
                fill="var(--text)"
                font-size="18"
                font-weight="700"
              >
                {tag ? `${formatValue(tag.value)} ${tag.unit_text}`.trim() : "--"}
              </text>
            {:else if w.kind === "label"}
              <text
                x={w.width / 2}
                y={w.height / 2 + 6}
                text-anchor="middle"
                fill="var(--text)"
                font-size="15"
                font-weight="600"
              >
                {w.label}
              </text>
            {/if}
          </g>
        {/each}
      </svg>
    </div>

    {#if editing && editScreen}
      <aside class="graphic-inspector" aria-label={m.inspectorAria}>
        <h4>{m.screenProps}</h4>
        <label>
          {m.screenName}
          <input bind:value={editScreen.name} maxlength="64" />
        </label>
        <div class="row-2">
          <label>
            {m.widthPx}
            <input type="number" min="200" max="3840" bind:value={editScreen.width} />
          </label>
          <label>
            {m.heightPx}
            <input type="number" min="120" max="2160" bind:value={editScreen.height} />
          </label>
        </div>

        {#if selectedWidget}
          <h4>{m.selectedWidget(selectedWidget.id)}</h4>
          <label>
            {m.displayName}
            <input bind:value={selectedWidget.label} maxlength="64" />
          </label>
          <label>
            {m.kind}
            <select bind:value={selectedWidget.kind}>
              <option value="tank">{m.widgetKind.tank}</option>
              <option value="gauge">{m.widgetKind.gauge}</option>
              <option value="pump">{m.widgetKind.pump}</option>
              <option value="valve">{m.widgetKind.valve}</option>
              <option value="lamp">{m.widgetKind.lamp}</option>
              <option value="value">{m.widgetKind.value}</option>
              <option value="label">{m.widgetKind.label}</option>
            </select>
          </label>
          {#if selectedWidget.kind !== "label"}
            <label>
              {m.linkedTag}
              <select bind:value={selectedWidget.tag}>
                <option value={null}>{m.notLinked}</option>
                {#each store.tags as t (t.name)}
                  <option value={t.name}>{t.name} ({t.kind})</option>
                {/each}
              </select>
            </label>
          {/if}
          <div class="row-2">
            <label>
              {m.xCoord}
              <input type="number" min="0" bind:value={selectedWidget.x} />
            </label>
            <label>
              {m.yCoord}
              <input type="number" min="0" bind:value={selectedWidget.y} />
            </label>
          </div>
          <div class="row-2">
            <label>
              {m.widgetWidth}
              <input type="number" min="20" bind:value={selectedWidget.width} />
            </label>
            <label>
              {m.widgetHeight}
              <input type="number" min="20" bind:value={selectedWidget.height} />
            </label>
          </div>
          {#if selectedWidget.kind === "tank" || selectedWidget.kind === "gauge"}
            <div class="row-2">
              <label>
                {m.minLabel}
                <input type="number" step="any" bind:value={selectedWidget.min} />
              </label>
              <label>
                {m.maxLabel}
                <input type="number" step="any" bind:value={selectedWidget.max} />
              </label>
            </div>
          {/if}
          <button type="button" class="danger-btn" onclick={removeSelectedWidget}>
            {m.deleteWidget}
          </button>
        {:else}
          <p class="muted">{m.canvasHint}</p>
        {/if}
      </aside>
    {/if}
  </div>
</section>

<style>
  .graphic-panel {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    min-height: 0;
  }
  .graphic-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 6px 14px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .tabs,
  .editor-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  button,
  input,
  select {
    font: inherit;
    font-size: 12px;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
    color: var(--text);
  }
  button {
    cursor: pointer;
  }
  button.active,
  button.primary-btn {
    border-color: var(--accent, #4ea1ff);
    font-weight: 600;
  }
  button.danger-btn {
    border-color: var(--danger, #ff5d5d);
    color: var(--danger, #ff5d5d);
  }
  .snap-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
  .graphic-status {
    margin: 0;
    padding: 4px 14px;
    font-size: 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .graphic-workspace {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-height: 0;
  }
  .graphic-workspace.with-inspector {
    grid-template-columns: minmax(0, 1fr) 260px;
  }
  .canvas-wrap {
    padding: 10px 14px;
    overflow: auto;
    max-height: 36vh;
  }
  .graphic-svg {
    width: 100%;
    max-width: 920px;
    height: auto;
    max-height: 32vh;
    background: var(--bg, #0f141c);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    display: block;
    user-select: none;
  }
  .graphic-inspector {
    padding: 10px 12px;
    background: var(--surface);
    border-left: 1px solid var(--border);
    display: grid;
    gap: 8px;
    align-content: start;
    max-height: 36vh;
    overflow: auto;
    font-size: 12px;
  }
  .graphic-inspector h4 {
    margin: 4px 0 0;
    font-size: 12px;
  }
  .graphic-inspector label {
    display: grid;
    gap: 3px;
  }
  .row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .muted {
    color: var(--muted);
    margin: 0;
  }
</style>
