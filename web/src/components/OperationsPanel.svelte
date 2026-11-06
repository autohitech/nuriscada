<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { onMount } from "svelte";
  import type { AlarmRecord, DeviceSpec, ImportReport, ProjectSpec, TagHistorySample, TagSpec } from "../lib/types";
  import { UndoStack } from "../lib/undo-stack";
  import { formatDateTime, formatImportCsv, formatTime, formatValue } from "../lib/format";
  import type { Store } from "../lib/store.svelte";
  import { eventLabel, m } from "../lib/i18n.svelte";
  import { apiErrorMessage } from "../lib/api";
  export interface SessionDoc {
    authentication_required: boolean;
    needs_setup?: boolean;
    writes_enabled: boolean;
    local_management?: boolean;
    user: { username: string; role: "admin" | "operator" | "viewer"; expires_ms: number } | null;
  }
  let { onSession, store }: { onSession: (session: SessionDoc) => void; store: Store } = $props();
  let session = $state<SessionDoc | null>(null);
  let username = $state("");
  let password = $state("");
  let message = $state("");
  let busy = $state(false);
  let targets = $state<{ name: string; kind: "bool" | "word" }[]>([]);
  let selected = $state("");
  let rawValue = $state("0");
  let pending = $state<{ name: string; value: number | boolean } | null>(null);
  let batchRows = $state<{ name: string; raw: string }[]>([]);
  let batchResults = $state<{ name: string; outcome: string }[] | null>(null);
  let pendingBatch = $state<{ name: string; value: number | boolean }[] | null>(null);
  let tagFilterText = $state("");
  const projectHistory = new UndoStack<ProjectSpec>(50);
  let projectUndoDepth = $state(0);
  let projectRedoDepth = $state(0);
  function syncProjectHistory(): void {
    projectUndoDepth = projectHistory.undoDepth;
    projectRedoDepth = projectHistory.redoDepth;
  }
  function snapshotProject(): ProjectSpec | null {
    return projectSpec === null ? null : $state.snapshot(projectSpec);
  }
  function commitProject(): void {
    const snap = snapshotProject();
    if (snap) projectHistory.commit(snap);
    syncProjectHistory();
  }
  function undoProject(): void {
    const current = snapshotProject();
    if (!current) return;
    const prev = projectHistory.undo(current);
    if (!prev) return;
    projectSpec = prev;
    syncProjectHistory();
  }
  function redoProject(): void {
    const current = snapshotProject();
    if (!current) return;
    const next = projectHistory.redo(current);
    if (!next) return;
    projectSpec = next;
    syncProjectHistory();
  }
  let events = $state<{ seq: number; event: { time_ms: number; actor: string; source: string; action: string; target: string; detail: unknown } }[]>([]);
  let newName = $state("");
  let newPassword = $state("");
  let newRole = $state("viewer");
  let history = $state<{id:number;data:AlarmRecord;ack_actor:string|null;ack_ms:number|null}[]>([]);
  let historyTag = $state("");
  let trendTag = $state("TANK_LEVEL");
  let trendLimit = $state(200);
  let trendSamples = $state<TagHistorySample[]>([]);
  let projectSpec = $state<ProjectSpec | null>(null);
  let projectToml = $state("");
  let importFormat = $state("simple");
  let importMode = $state("preview");
  let importDevice = $state("");
  let importText = $state("");
  let importReport = $state<ImportReport | null>(null);
  let importApplied = $state(false);
  let stopped = false;

  const trendStats = $derived.by(() => {
    const nums = trendSamples.map((s) => s.numeric).filter((v): v is number => v !== null && Number.isFinite(v));
    if (nums.length === 0) return null;
    const min = Math.min(...nums);
    const max = Math.max(...nums);
    const span = max - min || 1;
    const t0 = trendSamples[0]?.time_ms ?? 0;
    const t1 = trendSamples[trendSamples.length - 1]?.time_ms ?? t0;
    const dt = Math.max(t1 - t0, 1);
    const points = trendSamples
      .filter((s): s is TagHistorySample & { numeric: number } => s.numeric !== null && Number.isFinite(s.numeric))
      .map((s) => {
        const x = 16 + ((s.time_ms - t0) / dt) * 568;
        const y = max === min ? 60 : 104 - ((s.numeric - min) / span) * 88;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
    return { min, max, latest: nums[nums.length - 1], count: trendSamples.length, points };
  });

  async function loadTrend() {
    if (!trendTag.trim()) { message = m.enterTagName; return; }
    const res = await request(`history/tags?tag=${encodeURIComponent(trendTag.trim())}&limit=${trendLimit}`) as { samples: TagHistorySample[] };
    trendSamples = res.samples;
  }

  async function request(path: string, body?: unknown) {
    const response = await fetch(`/api/v1/${path}`, body === undefined ? { cache: "no-store" } : {
      method: "POST", headers: { "content-type": "application/json", "x-nuriscada-request": "1" }, body: JSON.stringify(body),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(apiErrorMessage(data, m.requestFailed(response.status)));
    return data;
  }

  async function loadProject() {
    const res = await request("project") as { project: ProjectSpec; toml: string };
    for (const t of res.project.tag) t.interlock ??= [];
    projectSpec = res.project;
    projectToml = res.toml;
    projectHistory.clear();
    syncProjectHistory();
  }
  function addDevice() {
    if (!projectSpec) return;
    commitProject();
    const idx = projectSpec.device.length + 1;
    const dev: DeviceSpec = { name: `dev${idx}`, driver: "modbus-tcp", address: "127.0.0.1:5020", timeout_ms: 500, scan_ms: 100 };
    projectSpec.device = [...projectSpec.device, dev];
  }

  function removeDevice(index: number) {
    if (!projectSpec) return;
    commitProject();
    projectSpec.device = projectSpec.device.filter((_, i) => i !== index);
  }

  function addTag() {
    if (!projectSpec) return;
    const idx = projectSpec.tag.length + 1;
    commitProject();
    const defaultDev = projectSpec.device[0]?.name ?? "sim";
    const tag: TagSpec = {
      name: `TAG_${idx}`,
      writable: false,
      device: defaultDev,
      unit: 1,
      space: "holding",
      address: 0,
      type: "analog",
      unit_text: "",
      value: null,
      length: null,
      swap_bytes: false,
      interlock: [],
      scale: { plc_base: 0, plc_full: 100, base: 0, full: 100, profile: 0, delay: 0, range: 0 },
      alarm: null,
    };
    projectSpec.tag = [...projectSpec.tag, tag];
  }

  function removeTag(index: number) {
    if (!projectSpec) return;
    commitProject();
    projectSpec.tag = projectSpec.tag.filter((_, i) => i !== index);
  }

  function onTagTypeChange(tag: TagSpec) {
    if (tag.type === "analog") {
      if (tag.space === "coil" || tag.space === "discrete" || tag.space === "m" || tag.space === "x" || tag.space === "y" || tag.space === "mx" || tag.space === "cio" || tag.space === "s7m") tag.space = "holding";
      if (!tag.scale) tag.scale = { plc_base: 0, plc_full: 100, base: 0, full: 100, profile: 0, delay: 0, range: 0 };
      tag.alarm = null;
      tag.value = null;
    } else if (tag.type === "digital") {
      if (tag.space === "holding" || tag.space === "input" || tag.space === "d" || tag.space === "w" || tag.space === "r" || tag.space === "mw" || tag.space === "dm" || tag.space === "s7db") tag.space = "coil";
      tag.scale = null;
      tag.unit_text = "";
      tag.alarm = null;
      tag.value = null;
    } else {
      tag.scale = null;
      tag.alarm = null;
      tag.writable = false;
      if (tag.value === null) tag.value = "";
      if (tag.space === "coil" || tag.space === "discrete") tag.space = "holding";
      if (tag.length === undefined) tag.length = null;
      if (tag.swap_bytes === undefined) tag.swap_bytes = false;
    }
  }

  function toggleTagAlarm(tag: TagSpec, enabled: boolean) {
    if (!enabled) {
      tag.alarm = null;
    } else if (tag.type === "analog") {
      tag.alarm = { kind: 6, hihi: 90, high: 70, low: 30, lolo: 10, gap: 5 };
    } else {
      tag.alarm = { kind: 0, hihi: null, high: null, low: null, lolo: null, gap: 0 };
    }
  }

  async function saveProject() {
    if (!projectSpec) return;
    const res = await request("project", projectSpec) as { project: ProjectSpec; toml: string };
    projectSpec = res.project;
    projectToml = res.toml;
    await refresh();
    message = m.projectSaved;
  }

  async function runImport() {
    const body: Record<string, unknown> = {
      text: importText,
      mode: importMode,
      format: importFormat,
    };
    if (importDevice.trim()) body.default_device = importDevice.trim();
    const res = await request("project/import", body) as { ok: boolean; applied: boolean; report: ImportReport };
    importReport = res.report;
    importApplied = res.applied;
    if (res.applied) {
      projectSpec = null;
      await loadProject();
      await refresh();
    }
  }

  async function readImportFile(file: File | undefined) {
    if (!file) return;
    importText = await file.text();
  }
  async function refresh() {
    try {
      const data = await request("auth/session") as SessionDoc;
      if (stopped) return;
      session = data; onSession(data);
      if (data.user && data.user.role !== "viewer") {
        targets = (await request("control/tags")).tags;
        if (!targets.some(t => t.name === selected)) selected = targets[0]?.name ?? "";
      } else { targets = []; events = []; pending = null; projectSpec = null; }
    } catch (e) { if (!stopped) message = String(e); }
  }

  async function perform(action: () => Promise<void>) {
    busy = true; message = "";
    try { await action(); } catch (e) { message = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }

  function login(event: SubmitEvent) {
    event.preventDefault();
    void perform(async () => {
      try { await request("auth/login", { username, password }); }
      finally { password = ""; }
      await refresh();
    });
  }

  function prepare(event: SubmitEvent) {
    event.preventDefault(); message = "";
    const target = targets.find(t => t.name === selected);
    if (!target) return;
    if (target.kind === "bool") pending = { name: selected, value: rawValue === "1" };
    else {
      if (!/^\d+$/.test(rawValue) || Number(rawValue) > 65535) { message = m.rawIntRange; return; }
      pending = { name: selected, value: Number(rawValue) };
    }
  }

  function addBatchRow() {
    const first = targets[0]?.name ?? "";
    batchRows = [...batchRows, { name: first, raw: "0" }];
    batchResults = null; pendingBatch = null;
  }

  function submitBatch(event: SubmitEvent) {
    event.preventDefault(); message = ""; batchResults = null; pendingBatch = null;
    if (batchRows.length === 0 || batchRows.length > 64) { message = m.rawIntRange; return; }
    const writes: { name: string; value: number | boolean }[] = [];
    for (const row of batchRows) {
      const target = targets.find(t => t.name === row.name);
      if (!target) { message = m.enterTagName; return; }
      if (target.kind === "bool") writes.push({ name: row.name, value: row.raw === "1" });
      else {
        if (!/^\d+$/.test(row.raw) || Number(row.raw) > 65535) { message = m.rawIntRange; return; }
        writes.push({ name: row.name, value: Number(row.raw) });
      }
    }
    pendingBatch = writes;
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 30000);
    return () => { stopped = true; clearInterval(timer); };
  });
</script>

<section class="operations" aria-label={m.accountsAndControl}>
  {#if !session}
    <p>{m.checkingServer}</p>
  {:else if !session.authentication_required}
    <p class="muted">{m.readOnlyDemo}</p>
  {:else if !session.user}
    {#if session.needs_setup}
      <p>{m.setupFirstAdmin}</p>
    {/if}
    <form onsubmit={login} class="inline">
      <label>{m.account} <input autocomplete="username" bind:value={username} maxlength="64" required /></label>
      <label>{m.password} <input type="password" autocomplete="current-password" bind:value={password} maxlength="128" required /></label>
      <button disabled={busy}>{m.login}</button>
    </form>
  {:else}
    <div class="inline">
      <span>{session.user.username} · {m.roles[session.user.role]}</span>
      <button disabled={busy} onclick={() => void perform(async () => { await request("auth/logout", {}); pending = null; events = []; projectSpec = null; await refresh(); })}>{m.logout}</button>
    </div>
    {#if session.user.role !== "viewer" && session.writes_enabled}
      <details>
        <summary>{m.writeTags}</summary>
        {#if targets.length === 0}<p>{m.noWritableTags}</p>
        {:else}
          <form class="inline" onsubmit={prepare}>
            <label>{m.target} <select bind:value={selected} onchange={() => { pending = null; rawValue = "0"; }}>
              {#each targets as target (target.name)}<option value={target.name}>{target.name}</option>{/each}
            </select></label>
            {#if targets.find(t => t.name === selected)?.kind === "bool"}
              <label>{m.colValue} <select bind:value={rawValue}><option value="0">OFF</option><option value="1">ON</option></select></label>
            {:else}<label>{m.rawRegisterValue} <input inputmode="numeric" bind:value={rawValue} required /></label>{/if}
            <button disabled={busy}>{m.reviewWrite}</button>
          </form>
          {#if pending}
            <div class="confirmation">
              <p>{m.writeConfirm(pending.name, String(pending.value))}</p>
              <button disabled={busy} onclick={() => void perform(async () => {
                const command = pending; pending = null;
                if (!command) return;
                await request(`control/tags/${encodeURIComponent(command.name)}`, { value: command.value });
                message = m.writeDone;
              })}>{m.confirmWrite}</button>
              <button disabled={busy} onclick={() => pending = null}>{m.cancel}</button>
            </div>
          {/if}
        {/if}
      </details>
      <details>
        <summary>{m.batchWrite}</summary>
        {#if targets.length === 0}<p>{m.noWritableTags}</p>
        {:else}
          <form onsubmit={submitBatch}>
            {#each batchRows as row, i (i)}
              <div class="inline">
                <label>{m.target} <select bind:value={row.name}>
                  {#each targets as target (target.name)}<option value={target.name}>{target.name}</option>{/each}
                </select></label>
                {#if targets.find(t => t.name === row.name)?.kind === "bool"}
                  <label>{m.colValue} <select bind:value={row.raw}><option value="0">OFF</option><option value="1">ON</option></select></label>
                {:else}<label>{m.rawRegisterValue} <input inputmode="numeric" bind:value={row.raw} required /></label>{/if}
                <button type="button" disabled={busy} onclick={() => { batchRows = batchRows.filter((_, j) => j !== i); batchResults = null; pendingBatch = null; }}>{m.removeTag}</button>
              </div>
            {/each}
            <div class="inline">
              <button type="button" disabled={busy} onclick={addBatchRow}>{m.addRow}</button>
              <button disabled={busy || batchRows.length === 0}>{m.reviewWrite}</button>
            </div>
          </form>
          {#if pendingBatch}
            <div class="confirmation">
              {#each pendingBatch as w, i (i)}<p>{m.writeConfirm(w.name, String(w.value))}</p>{/each}
              <button disabled={busy} onclick={() => void perform(async () => {
                const writes = pendingBatch; pendingBatch = null;
                if (!writes) return;
                const res = await request("control/tags/batch", { writes }) as { ok: boolean; results: { name: string; outcome: string }[] };
                batchRows = [];
                batchResults = res.results;
                message = res.ok ? m.batchDone : m.batchPartial;
              })}>{m.confirmWrite}</button>
              <button disabled={busy} onclick={() => pendingBatch = null}>{m.cancel}</button>
            </div>
          {/if}
          {#if batchResults}
            <div class="audit">
              {#each batchResults as r, i (i)}<p><strong>{r.name}</strong> · {m.batchOutcome(r.outcome)}</p>{/each}
            </div>
          {/if}
        {/if}
      </details>
    {:else}<p class="muted">{m.currentlyReadOnly}</p>{/if}
    {#if session.user.role === "admin" && session.local_management}
      <details>
        <summary>{m.editProject}</summary>
        <div class="inline">
          <button type="button" disabled={busy} onclick={() => void perform(loadProject)}>{m.loadCurrent}</button>
          {#if projectSpec}
            <button type="button" disabled={busy} onclick={addDevice}>{m.addDevice}</button>
            <button type="button" disabled={busy} onclick={addTag}>{m.addTag}</button>
            <button type="button" disabled={busy} onclick={() => void perform(saveProject)}>{m.saveApply}</button>
            <button type="button" disabled={busy || projectUndoDepth === 0} onclick={undoProject} title="Ctrl+Z">{m.undoEdit}</button>
            <button type="button" disabled={busy || projectRedoDepth === 0} onclick={redoProject} title="Ctrl+Shift+Z">{m.redoEdit}</button>
          {/if}
        </div>
        {#if projectSpec}
          <div class="editor-block">
            <div class="inline">
              <label>{m.listenAddress} <input bind:value={projectSpec.server.listen} required /></label>
              <label>{m.publishMs} <input type="number" min="20" bind:value={projectSpec.server.publish_ms} required /></label>
            </div>
            <h4>{m.devicesHeading(projectSpec.device.length)}</h4>
            {#each projectSpec.device as dev, di (di)}
              <div class="inline editor-row">
                <label>{m.deviceName} <input bind:value={dev.name} required /></label>
                <label>{m.driver} <select bind:value={dev.driver}><option value="modbus-tcp">modbus-tcp</option></select></label>
                <label>{m.hostPort} <input bind:value={dev.address} required /></label>
                <label>{m.timeoutMs} <input type="number" min="1" bind:value={dev.timeout_ms} required /></label>
                <label>{m.scanMs} <input type="number" min="10" bind:value={dev.scan_ms} required /></label>
                <button type="button" disabled={busy} onclick={() => removeDevice(di)}>{m.removeDevice}</button>
              </div>
            {/each}
            <h4>{m.tagsHeading(projectSpec.tag.length)}</h4>
            <div class="inline">
              <label>{m.tagFilter} <input bind:value={tagFilterText} maxlength="64" placeholder={m.tagFilterPlaceholder} /></label>
            </div>
            {#each projectSpec.tag as tag, ti (ti)}
              {#if tagFilterText.trim() === "" || tag.name.toLowerCase().includes(tagFilterText.trim().toLowerCase()) || tag.device.toLowerCase().includes(tagFilterText.trim().toLowerCase())}
              <div class="editor-row">
                <div class="inline">
                  <label>{m.tagName} <input bind:value={tag.name} required /></label>
                  <label>{m.colDevice} <input bind:value={tag.device} required /></label>
                  <label>{m.unitNo} <input type="number" min="0" max="255" bind:value={tag.unit} required /></label>
                  <label>{m.kind} <select bind:value={tag.type} onchange={() => onTagTypeChange(tag)}>
                    <option value="analog">analog</option>
                    <option value="digital">digital</option>
                    <option value="string">string</option>
                  </select></label>
                  <label>{m.space} <select bind:value={tag.space}>
                    {#if tag.type === "analog" || tag.type === "string"}
                      <option value="holding">holding</option>
                      <option value="input">input</option>
                      <option value="d">d</option>
                      <option value="w">w</option>
                      <option value="r">r</option>
                      <option value="mw">mw</option>
                      <option value="dm">dm</option>
                      <option value="s7db">s7db</option>
                    {:else if tag.type === "digital"}
                      <option value="coil">coil</option>
                      <option value="discrete">discrete</option>
                      <option value="m">m</option>
                      <option value="x">x</option>
                      <option value="y">y</option>
                      <option value="mx">mx</option>
                      <option value="cio">cio</option>
                      <option value="wr">wr</option>
                      <option value="hr">hr</option>
                      <option value="ar">ar</option>
                      <option value="s7m">s7m</option>
                    {/if}
                  </select></label>
                  <label>{m.address} <input type="number" min="0" max="65535" bind:value={tag.address} required /></label>
                  {#if tag.type !== "string"}
                    <label class="checkbox-label"><input type="checkbox" bind:checked={tag.writable} /> {m.allowWrite}</label>
                  {/if}
                  {#if tag.type === "analog" || tag.type === "string"}
                    <label>{m.unitText} <input bind:value={tag.unit_text} placeholder="%" /></label>
                  {/if}
                  {#if tag.type === "string"}
                    <label>{m.tagString} <input bind:value={tag.value} /></label>
                    <label>{m.stringLength} <input type="number" min="1" max="32" value={tag.length ?? ""} onchange={(e) => { const v = (e.currentTarget as HTMLInputElement).value; tag.length = v === "" ? null : Number(v); }} /></label>
                    <label class="checkbox-label"><input type="checkbox" bind:checked={tag.swap_bytes} /> {m.swapBytes}</label>
                  {/if}
                  {#if tag.type !== "string"}
                    <label class="checkbox-label"><input type="checkbox" checked={tag.alarm !== null} onchange={(e) => toggleTagAlarm(tag, (e.currentTarget as HTMLInputElement).checked)} /> {m.useAlarm}</label>
                  {/if}
                  <button type="button" disabled={busy} onclick={() => removeTag(ti)}>{m.removeTag}</button>
                </div>
                {#if tag.type === "analog" && tag.scale}
                  <div class="inline sub-row">
                    <label>{m.plcBase} <input type="number" step="any" bind:value={tag.scale.plc_base} /></label>
                    <label>{m.plcFull} <input type="number" step="any" bind:value={tag.scale.plc_full} /></label>
                    <label>{m.dispBase} <input type="number" step="any" bind:value={tag.scale.base} /></label>
                    <label>{m.dispFull} <input type="number" step="any" bind:value={tag.scale.full} /></label>
                    <label>{m.profile} <select bind:value={tag.scale.profile}>
                      <option value={0}>{m.profileLinear}</option>
                      <option value={1}>{m.profileCosine}</option>
                      <option value={3}>{m.profileMirror}</option>
                      <option value={4}>{m.profilePass}</option>
                    </select></label>
                    <label>{m.delay} <input type="number" min="0" max="1000" bind:value={tag.scale.delay} /></label>
                    <label>{m.rangeMode} <select bind:value={tag.scale.range}>
                      <option value={0}>{m.rangeNone}</option>
                      <option value={1}>{m.rangeClamp}</option>
                      <option value={2}>{m.rangeDrop}</option>
                    </select></label>
                  </div>
                {/if}
                {#if tag.alarm}
                  <div class="inline sub-row">
                    {#if tag.type === "analog"}
                      <label>{m.alarmKind} <input type="number" min="0" max="9" bind:value={tag.alarm.kind} /></label>
                      <label>HIHI <input type="number" step="any" bind:value={tag.alarm.hihi} /></label>
                      <label>HIGH <input type="number" step="any" bind:value={tag.alarm.high} /></label>
                      <label>LOW <input type="number" step="any" bind:value={tag.alarm.low} /></label>
                      <label>LOLO <input type="number" step="any" bind:value={tag.alarm.lolo} /></label>
                      <label>{m.alarmGap} <input type="number" step="any" bind:value={tag.alarm.gap} /></label>
                    {:else}
                      <label>{m.digitalAlarmKind} <input type="number" min="0" max="3" bind:value={tag.alarm.kind} /></label>
                    {/if}
                  </div>
                {/if}
                {#if tag.writable}
                  <div class="inline sub-row">
                    <span>{m.interlockConds}</span>
                    {#each tag.interlock as cond, ci (ci)}
                      <select bind:value={cond.tag}>
                        {#each (projectSpec?.tag ?? []).filter((t2) => t2.name !== tag.name) as cand (cand.name)}
                          <option value={cand.name}>{cand.name}</option>
                        {/each}
                      </select>
                      <select value={typeof cond.value === "boolean" ? "bool" : "num"} onchange={(e) => { const k = (e.currentTarget as HTMLSelectElement).value; cond.value = k === "bool" ? false : 0; tag.interlock = [...tag.interlock]; }}>
                        <option value="bool">{m.condBool}</option>
                        <option value="num">{m.condNum}</option>
                      </select>
                      {#if typeof cond.value === "boolean"}
                        <input type="checkbox" bind:checked={cond.value} />
                      {:else}
                        <input type="number" step="1" value={cond.value} onchange={(e) => { cond.value = Number((e.currentTarget as HTMLInputElement).value); tag.interlock = [...tag.interlock]; }} />
                      {/if}
                      {@const live = store.tags.find((t) => t.name === cond.tag)}
                      <span class="muted" title={live?.quality === "bad" ? m.badQuality : ""}>→ {live ? formatValue(live.value) : "—"} {live && live.quality === "good" && ((typeof cond.value === "boolean" && typeof live.value === "boolean" && cond.value === live.value) || (typeof cond.value === "number" && typeof live.value === "number" && cond.value === live.value)) ? "✓" : "✗"}</span>
                      <button type="button" disabled={busy} onclick={() => { tag.interlock = tag.interlock.filter((_, j) => j !== ci); }}>{m.removeCond}</button>
                    {/each}
                    <button type="button" disabled={busy} onclick={() => { const first = (projectSpec?.tag ?? []).find((t2) => t2.name !== tag.name); if (first) tag.interlock = [...tag.interlock, { tag: first.name, value: false }]; }}>{m.addRow}</button>
                  </div>
                {/if}
              </div>
              {/if}
            {/each}
            {#if projectToml}
              <details>
                <summary>{m.tomlPreview}</summary>
                <pre class="toml-preview">{projectToml}</pre>
              </details>
            {/if}
          </div>
        {/if}
      </details>
      <details>
        <summary>{m.importHeading}</summary>
        <div class="inline">
          <label>{m.importFormat} <select bind:value={importFormat}>
            <option value="simple">{m.importSimple}</option>
            <option value="tagx">{m.importTagx}</option>
          </select></label>
          <label>{m.importMode} <select bind:value={importMode}>
            <option value="preview">{m.importPreview}</option>
            <option value="merge">{m.importMerge}</option>
            <option value="replace">{m.importReplace}</option>
          </select></label>
          <label>{m.importDevice} <input bind:value={importDevice} placeholder="sim" /></label>
          <label>{m.importFile} <input type="file" accept=".txt,.csv,.tagx,text/plain" onchange={(e) => void readImportFile((e.currentTarget as HTMLInputElement).files?.[0])} /></label>
          <button type="button" disabled={busy || !importText.trim()} onclick={() => void perform(runImport)}>{m.importRun}</button>
          {#if importReport}
            <button type="button" onclick={() => {
              const report = importReport;
              if (!report) return;
              const blob = new Blob(["\uFEFF" + formatImportCsv(report)], { type: "text/csv;charset=utf-8" });
              const url = URL.createObjectURL(blob);
              const a = document.createElement("a");
              a.href = url;
              a.download = "import-report.csv";
              a.click();
              URL.revokeObjectURL(url);
            }}>{m.importCsv}</button>
          {/if}
        </div>
        <label>{m.importText} <textarea rows="6" bind:value={importText}></textarea></label>
        {#if importReport}
          <p>{m.importSummary(importReport.imported_devices, importReport.imported_tags)} · {importApplied ? m.importApplied : m.importNotApplied}</p>
          {#if importReport.renamed_tags.length > 0}
            <details>
              <summary>renamed ({importReport.renamed_tags.length})</summary>
              <ul>
                {#each importReport.renamed_tags as it}
                  <li>{it.line}: {it.original} → {it.resolved}</li>
                {/each}
              </ul>
            </details>
          {/if}
          {#if importReport.modified_items.length > 0}
            <details><summary>modified ({importReport.modified_items.length})</summary><ul>
              {#each importReport.modified_items as it}<li>{it.line}: {it.item} — {it.detail}</li>{/each}
            </ul></details>
          {/if}
          {#if importReport.ignored_items.length > 0}
            <details><summary>ignored ({importReport.ignored_items.length})</summary><ul>
              {#each importReport.ignored_items as it}<li>{it.line}: {it.item} — {it.detail}</li>{/each}
            </ul></details>
          {/if}
          {#if importReport.unreadable_items.length > 0}
            <details><summary>unreadable ({importReport.unreadable_items.length})</summary><ul>
              {#each importReport.unreadable_items as it}<li>{it.line}: {it.item} — {it.detail}</li>{/each}
            </ul></details>
          {/if}
        {/if}
      </details>
      <details>
        <summary>{m.addAccount}</summary>
        <form class="inline" onsubmit={(event) => { event.preventDefault(); void perform(async () => {
          try { await request("users", { username: newName, password: newPassword, role: newRole }); }
          finally { newPassword = ""; }
          message = m.accountAdded; newName = "";
        }); }}>
          <label>{m.newAccount} <input bind:value={newName} autocomplete="off" maxlength="64" required /></label>
          <label>{m.newPassword} <input type="password" bind:value={newPassword} autocomplete="new-password" minlength="12" maxlength="128" required /></label>
          <label>{m.roleLabel} <select bind:value={newRole}><option value="viewer">{m.roles.viewer}</option><option value="operator">{m.roles.operator}</option><option value="admin">{m.roles.admin}</option></select></label>
          <button disabled={busy}>{m.addAccount}</button>
        </form>
      </details>
      <details>
        <summary>{m.auditLog}</summary>
        <button disabled={busy} onclick={() => void perform(async () => { events = (await request("audit")).events; })}>{m.fromStart}</button>
        <button disabled={busy || events.length === 0} onclick={() => void perform(async () => { events = (await request(`audit?after=${events[events.length - 1]?.seq ?? 0}`)).events; })}>{m.nextPage}</button>
        <div class="audit">
          {#each events as record (record.seq)}
            <p><strong>#{record.seq} {record.event.action}</strong> · {new Date(record.event.time_ms).toLocaleString()} · {record.event.actor} · {record.event.target}<br />{record.event.source} · {JSON.stringify(record.event.detail)}</p>
          {/each}
        </div>
      </details>
    {/if}
    <details>
      <summary>{m.alarmHistory}</summary>
      <div class="inline">
        <label>{m.tagFilter} <input bind:value={historyTag} maxlength="256" /></label>
        <button disabled={busy} onclick={() => void perform(async () => { history = (await request(historyTag ? "history/alarms?tag=" + encodeURIComponent(historyTag) : "history/alarms")).records; })}>{m.recentRecords}</button>
        <button disabled={busy || history.length === 0} onclick={() => void perform(async () => {
          history = (await request(`history/alarms?before=${history[history.length - 1]?.id ?? 0}${historyTag ? `&tag=${encodeURIComponent(historyTag)}` : ""}`)).records;
        })}>{m.olderRecords}</button>
      </div>
      <p class="muted">{m.ackNote}</p>
      <div class="audit">
        {#each history as record (record.id)}
          <p>
            <strong>#{record.id} {record.data.tag} · {eventLabel(record.data.event)}</strong> · {formatDateTime(record.data.time_ms)} · {record.data.value}
            {#if record.ack_actor}
              <span> · {m.ackBy}: {record.ack_actor} ({formatDateTime(record.ack_ms ?? 0)})</span>
            {:else if session.local_management && session.user.role !== "viewer" && record.data.event !== "RETURN" && record.data.event !== "CNF"}
              <button disabled={busy} onclick={() => void perform(async () => {
                await request(`history/alarms/${record.id}/ack`, {});
                history = (await request("history/alarms")).records;
                message = m.alarmAcked;
              })}>{m.ack}</button>
            {/if}
          </p>
        {/each}
      </div>
    </details>
    <details>
      <summary>{m.trendSection}</summary>
      <div class="inline">
        <label>{m.tagName} <input bind:value={trendTag} maxlength="256" required /></label>
        <label>{m.maxSamples} <input type="number" min="10" max="2000" bind:value={trendLimit} /></label>
        <button type="button" disabled={busy} onclick={() => void perform(loadTrend)}>{m.loadTrend}</button>
        {#if trendTag.trim()}
          <a class="csv-link" href={`/api/v1/history/tags?tag=${encodeURIComponent(trendTag.trim())}&limit=${trendLimit}&format=csv`} download={`${trendTag.trim()}-history.csv`}>{m.downloadCsv}</a>
        {/if}
      </div>
      {#if trendStats}
        <p class="muted">{m.trendStats(trendStats.count, String(trendStats.min), String(trendStats.max), String(trendStats.latest))}</p>
        <svg class="trend-chart" viewBox="0 0 600 120" role="img" aria-label={m.trendAria(trendTag)}>
          <line x1="16" y1="16" x2="584" y2="16" stroke="var(--border)" stroke-dasharray="3,3" />
          <line x1="16" y1="60" x2="584" y2="60" stroke="var(--border)" stroke-dasharray="3,3" />
          <line x1="16" y1="104" x2="584" y2="104" stroke="var(--border)" />
          {#if trendStats.points}
            <polyline fill="none" stroke="var(--accent, #4ea1ff)" stroke-width="2" points={trendStats.points} />
          {/if}
        </svg>
        <div class="audit">
          {#each trendSamples.slice(-10).reverse() as sample (sample.id)}
            <p><strong>#{sample.id}</strong> · {formatTime(sample.time_ms)} · {m.valueLabel}: {String(sample.value)} · {m.qualityLabel}: {sample.quality}{sample.alarm_state ? ` · ${m.alarmLabel}: ${sample.alarm_state}` : ""}</p>
          {/each}
        </div>
      {:else if trendSamples.length === 0}
        <p class="muted">{m.trendHint}</p>
      {/if}
    </details>
  {/if}
  <p role="status" class="message">{message}</p>
</section>

<style>
  .operations { padding: 8px 16px; background: var(--surface); border-bottom: 1px solid var(--border); max-height: 45vh; overflow: auto; }
  .inline { display: flex; gap: 10px; flex-wrap: wrap; align-items: end; }
  label { display: grid; gap: 4px; font-size: 12px; }
  .checkbox-label { display: flex; align-items: center; gap: 6px; padding-bottom: 6px; }
  input, select, button, .csv-link { font: inherit; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius); color: var(--text); background: var(--surface-2); text-decoration: none; }
  button { cursor: pointer; } button:disabled { opacity: .5; cursor: default; }
  summary { cursor: pointer; padding: 6px 0; } p { margin: 6px 0; } .message { white-space: pre-wrap; }
  .muted { color: var(--muted); font-size: 12px; } .audit { overflow-wrap: anywhere; font-size: 12px; }
  .confirmation { border-left: 3px solid var(--warn); padding: 8px; margin: 8px 0; }
  .editor-block { display: grid; gap: 8px; margin-top: 8px; }
  .editor-block h4 { margin: 4px 0 0; font-size: 13px; }
  .editor-row { border: 1px solid var(--border); border-radius: var(--radius); padding: 8px; display: grid; gap: 6px; }
  .sub-row { padding-top: 4px; border-top: 1px dashed var(--border); }
  .toml-preview { font-size: 12px; background: var(--surface-2); padding: 8px; border-radius: var(--radius); overflow: auto; max-height: 180px; }
  .trend-chart { width: 100%; max-width: 600px; height: 120px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius); margin: 6px 0; }
</style>
