<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { onMount } from "svelte";
  import type { AlarmRecord, DeviceSpec, ProjectSpec, TagSpec } from "../lib/types";
  import { alarmEventLabel } from "../lib/strings";
  export interface SessionDoc {
    authentication_required: boolean;
    needs_setup?: boolean;
    writes_enabled: boolean;
    local_management?: boolean;
    user: { username: string; role: "admin" | "operator" | "viewer"; expires_ms: number } | null;
  }
  let { onSession }: { onSession: (session: SessionDoc) => void } = $props();
  let session = $state<SessionDoc | null>(null);
  let username = $state("");
  let password = $state("");
  let message = $state("");
  let busy = $state(false);
  let targets = $state<{ name: string; kind: "bool" | "word" }[]>([]);
  let selected = $state("");
  let rawValue = $state("0");
  let pending = $state<{ name: string; value: number | boolean } | null>(null);
  let events = $state<{ seq: number; event: { time_ms: number; actor: string; source: string; action: string; target: string; detail: unknown } }[]>([]);
  let newName = $state("");
  let newPassword = $state("");
  let newRole = $state("viewer");
  let history = $state<{id:number;data:AlarmRecord;ack_actor:string|null;ack_ms:number|null}[]>([]);
  let historyTag = $state("");
  let projectSpec = $state<ProjectSpec | null>(null);
  let projectToml = $state("");
  let stopped = false;

  async function request(path: string, body?: unknown) {
    const response = await fetch(`/api/v1/${path}`, body === undefined ? { cache: "no-store" } : {
      method: "POST", headers: { "content-type": "application/json", "x-nuriscada-request": "1" }, body: JSON.stringify(body),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error ?? `요청 실패 (${response.status})`);
    return data;
  }

  async function loadProject() {
    const res = await request("project") as { project: ProjectSpec; toml: string };
    projectSpec = res.project;
    projectToml = res.toml;
  }

  function addDevice() {
    if (!projectSpec) return;
    const idx = projectSpec.device.length + 1;
    const dev: DeviceSpec = { name: `dev${idx}`, driver: "modbus-tcp", address: "127.0.0.1:5020", timeout_ms: 500, scan_ms: 100 };
    projectSpec.device = [...projectSpec.device, dev];
  }

  function removeDevice(index: number) {
    if (!projectSpec) return;
    projectSpec.device = projectSpec.device.filter((_, i) => i !== index);
  }

  function addTag() {
    if (!projectSpec) return;
    const idx = projectSpec.tag.length + 1;
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
      scale: { plc_base: 0, plc_full: 100, base: 0, full: 100, profile: 0, delay: 0, range: 0 },
      alarm: null,
    };
    projectSpec.tag = [...projectSpec.tag, tag];
  }

  function removeTag(index: number) {
    if (!projectSpec) return;
    projectSpec.tag = projectSpec.tag.filter((_, i) => i !== index);
  }

  function onTagTypeChange(tag: TagSpec) {
    if (tag.type === "analog") {
      if (tag.space === "coil" || tag.space === "discrete") tag.space = "holding";
      if (!tag.scale) tag.scale = { plc_base: 0, plc_full: 100, base: 0, full: 100, profile: 0, delay: 0, range: 0 };
      tag.alarm = null;
    } else {
      if (tag.space === "holding" || tag.space === "input") tag.space = "coil";
      tag.scale = null;
      tag.unit_text = "";
      tag.alarm = null;
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
    message = "프로젝트 설정을 저장하고 스캐너에 즉시 적용했습니다.";
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
      if (!/^\d+$/.test(rawValue) || Number(rawValue) > 65535) { message = "원시 정수 0~65535를 입력하세요."; return; }
      pending = { name: selected, value: Number(rawValue) };
    }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 30000);
    return () => { stopped = true; clearInterval(timer); };
  });
</script>

<section class="operations" aria-label="계정과 조작">
  {#if !session}
    <p>서버 연결 확인 중…</p>
  {:else if !session.authentication_required}
    <p class="muted">읽기 전용 시연</p>
  {:else if !session.user}
    {#if session.needs_setup}
      <p>서버 터미널에서 최초 관리자를 설정한 뒤 로그인하세요.</p>
    {/if}
    <form onsubmit={login} class="inline">
      <label>계정 <input autocomplete="username" bind:value={username} maxlength="64" required /></label>
      <label>비밀번호 <input type="password" autocomplete="current-password" bind:value={password} maxlength="128" required /></label>
      <button disabled={busy}>로그인</button>
    </form>
  {:else}
    <div class="inline">
      <span>{session.user.username} · {{ admin: "관리자", operator: "운전원", viewer: "조회자" }[session.user.role]}</span>
      <button disabled={busy} onclick={() => void perform(async () => { await request("auth/logout", {}); pending = null; events = []; projectSpec = null; await refresh(); })}>로그아웃</button>
    </div>
    {#if session.user.role !== "viewer" && session.writes_enabled}
      <details>
        <summary>태그 쓰기</summary>
        {#if targets.length === 0}<p>쓰기가 허용된 태그가 없습니다.</p>
        {:else}
          <form class="inline" onsubmit={prepare}>
            <label>대상 <select bind:value={selected} onchange={() => { pending = null; rawValue = "0"; }}>
              {#each targets as target (target.name)}<option value={target.name}>{target.name}</option>{/each}
            </select></label>
            {#if targets.find(t => t.name === selected)?.kind === "bool"}
              <label>값 <select bind:value={rawValue}><option value="0">OFF</option><option value="1">ON</option></select></label>
            {:else}<label>원시 레지스터 값 <input inputmode="numeric" bind:value={rawValue} required /></label>{/if}
            <button disabled={busy}>쓰기 내용 확인</button>
          </form>
          {#if pending}
            <div class="confirmation">
              <p><strong>{pending.name}</strong>에 <strong>{String(pending.value)}</strong>을 씁니다. 레지스터 값은 환산 전 원시 값입니다.</p>
              <button disabled={busy} onclick={() => void perform(async () => {
                const command = pending; pending = null;
                if (!command) return;
                await request(`control/tags/${encodeURIComponent(command.name)}`, { value: command.value });
                message = "장치 응답을 확인했습니다. 표시값은 다음 스캔에서 갱신됩니다.";
              })}>확인 후 쓰기</button>
              <button disabled={busy} onclick={() => pending = null}>취소</button>
            </div>
          {/if}
        {/if}
      </details>
    {:else}<p class="muted">현재 읽기 전용입니다.</p>{/if}
    {#if session.user.role === "admin" && session.local_management}
      <details>
        <summary>프로젝트·태그·통신 설정 편집</summary>
        <div class="inline">
          <button type="button" disabled={busy} onclick={() => void perform(loadProject)}>현재 설정 불러오기</button>
          {#if projectSpec}
            <button type="button" disabled={busy} onclick={addDevice}>통신 장치 추가</button>
            <button type="button" disabled={busy} onclick={addTag}>태그 추가</button>
            <button type="button" disabled={busy} onclick={() => void perform(saveProject)}>설정 저장 및 즉시 적용</button>
          {/if}
        </div>
        {#if projectSpec}
          <div class="editor-block">
            <div class="inline">
              <label>수신 주소 <input bind:value={projectSpec.server.listen} required /></label>
              <label>방송 주기(ms) <input type="number" min="20" bind:value={projectSpec.server.publish_ms} required /></label>
            </div>
            <h4>통신 장치 ({projectSpec.device.length})</h4>
            {#each projectSpec.device as dev, di (di)}
              <div class="inline editor-row">
                <label>장치 이름 <input bind:value={dev.name} required /></label>
                <label>드라이버 <select bind:value={dev.driver}><option value="modbus-tcp">modbus-tcp</option></select></label>
                <label>주소(host:port) <input bind:value={dev.address} required /></label>
                <label>제한 시간(ms) <input type="number" min="1" bind:value={dev.timeout_ms} required /></label>
                <label>스캔 주기(ms) <input type="number" min="10" bind:value={dev.scan_ms} required /></label>
                <button type="button" disabled={busy} onclick={() => removeDevice(di)}>장치 삭제</button>
              </div>
            {/each}
            <h4>태그 ({projectSpec.tag.length})</h4>
            {#each projectSpec.tag as tag, ti (ti)}
              <div class="editor-row">
                <div class="inline">
                  <label>태그 이름 <input bind:value={tag.name} required /></label>
                  <label>장치 <input bind:value={tag.device} required /></label>
                  <label>유닛 <input type="number" min="0" max="255" bind:value={tag.unit} required /></label>
                  <label>종류 <select bind:value={tag.type} onchange={() => onTagTypeChange(tag)}>
                    <option value="analog">analog</option>
                    <option value="digital">digital</option>
                  </select></label>
                  <label>영역 <select bind:value={tag.space}>
                    {#if tag.type === "analog"}
                      <option value="holding">holding</option>
                      <option value="input">input</option>
                    {:else}
                      <option value="coil">coil</option>
                      <option value="discrete">discrete</option>
                    {/if}
                  </select></label>
                  <label>주소 <input type="number" min="0" max="65535" bind:value={tag.address} required /></label>
                  <label class="checkbox-label"><input type="checkbox" bind:checked={tag.writable} /> 쓰기 허용</label>
                  {#if tag.type === "analog"}
                    <label>단위 <input bind:value={tag.unit_text} placeholder="%" /></label>
                  {/if}
                  <label class="checkbox-label"><input type="checkbox" checked={tag.alarm !== null} onchange={(e) => toggleTagAlarm(tag, (e.currentTarget as HTMLInputElement).checked)} /> 알람 사용</label>
                  <button type="button" disabled={busy} onclick={() => removeTag(ti)}>태그 삭제</button>
                </div>
                {#if tag.type === "analog" && tag.scale}
                  <div class="inline sub-row">
                    <label>PLC 하한 <input type="number" step="any" bind:value={tag.scale.plc_base} /></label>
                    <label>PLC 상한 <input type="number" step="any" bind:value={tag.scale.plc_full} /></label>
                    <label>표시 하한 <input type="number" step="any" bind:value={tag.scale.base} /></label>
                    <label>표시 상한 <input type="number" step="any" bind:value={tag.scale.full} /></label>
                    <label>프로파일 <select bind:value={tag.scale.profile}>
                      <option value={0}>0 (선형)</option>
                      <option value={1}>1 (코사인)</option>
                      <option value={3}>3 (구간 반전)</option>
                      <option value={4}>4 (무변환)</option>
                    </select></label>
                    <label>지연(0~1000) <input type="number" min="0" max="1000" bind:value={tag.scale.delay} /></label>
                    <label>범위 처리 <select bind:value={tag.scale.range}>
                      <option value={0}>0 (없음)</option>
                      <option value={1}>1 (자름)</option>
                      <option value={2}>2 (버림)</option>
                    </select></label>
                  </div>
                {/if}
                {#if tag.alarm}
                  <div class="inline sub-row">
                    {#if tag.type === "analog"}
                      <label>알람 종류(0~9) <input type="number" min="0" max="9" bind:value={tag.alarm.kind} /></label>
                      <label>HIHI <input type="number" step="any" bind:value={tag.alarm.hihi} /></label>
                      <label>HIGH <input type="number" step="any" bind:value={tag.alarm.high} /></label>
                      <label>LOW <input type="number" step="any" bind:value={tag.alarm.low} /></label>
                      <label>LOLO <input type="number" step="any" bind:value={tag.alarm.lolo} /></label>
                      <label>복귀 여유(gap) <input type="number" step="any" bind:value={tag.alarm.gap} /></label>
                    {:else}
                      <label>디지털 알람 종류(0~3) <input type="number" min="0" max="3" bind:value={tag.alarm.kind} /></label>
                    {/if}
                  </div>
                {/if}
              </div>
            {/each}
            {#if projectToml}
              <details>
                <summary>생성된 TOML 미리보기</summary>
                <pre class="toml-preview">{projectToml}</pre>
              </details>
            {/if}
          </div>
        {/if}
      </details>
      <details>
        <summary>계정 추가</summary>
        <form class="inline" onsubmit={(event) => { event.preventDefault(); void perform(async () => {
          try { await request("users", { username: newName, password: newPassword, role: newRole }); }
          finally { newPassword = ""; }
          message = "계정을 추가했습니다."; newName = "";
        }); }}>
          <label>새 계정 <input bind:value={newName} autocomplete="off" maxlength="64" required /></label>
          <label>새 비밀번호 <input type="password" bind:value={newPassword} autocomplete="new-password" minlength="12" maxlength="128" required /></label>
          <label>역할 <select bind:value={newRole}><option value="viewer">조회자</option><option value="operator">운전원</option><option value="admin">관리자</option></select></label>
          <button disabled={busy}>계정 추가</button>
        </form>
      </details>
      <details>
        <summary>감사 기록</summary>
        <button disabled={busy} onclick={() => void perform(async () => { events = (await request("audit")).events; })}>처음부터 조회</button>
        <button disabled={busy || events.length === 0} onclick={() => void perform(async () => { events = (await request(`audit?after=${events[events.length - 1]?.seq ?? 0}`)).events; })}>다음 페이지</button>
        <div class="audit">
          {#each events as record (record.seq)}
            <p><strong>#{record.seq} {record.event.action}</strong> · {new Date(record.event.time_ms).toLocaleString()} · {record.event.actor} · {record.event.target}<br />{record.event.source} · {JSON.stringify(record.event.detail)}</p>
          {/each}
        </div>
      </details>
    {/if}
    <details>
      <summary>알람 이력·확인</summary>
      <div class="inline">
        <label>태그 필터 <input bind:value={historyTag} maxlength="256" /></label>
        <button disabled={busy} onclick={() => void perform(async () => { history = (await request(historyTag ? "history/alarms?tag=" + encodeURIComponent(historyTag) : "history/alarms")).records; })}>최근 기록 조회</button>
        <button disabled={busy || history.length === 0} onclick={() => void perform(async () => {
          history = (await request(`history/alarms?before=${history[history.length - 1]?.id ?? 0}${historyTag ? `&tag=${encodeURIComponent(historyTag)}` : ""}`)).records;
        })}>이전 기록</button>
      </div>
      <p class="muted">기록 확인은 알람 정상 복귀와 다릅니다.</p>
      <div class="audit">
        {#each history as record (record.id)}
          <p>
            <strong>#{record.id} {record.data.tag} · {alarmEventLabel[record.data.event]}</strong> · {new Date(record.data.time_ms).toLocaleString()} · {record.data.value}
            {#if record.ack_actor}
              <span> · 확인: {record.ack_actor} ({new Date(record.ack_ms ?? 0).toLocaleString()})</span>
            {:else if session.local_management && session.user.role !== "viewer" && record.data.event !== "RETURN" && record.data.event !== "CNF"}
              <button disabled={busy} onclick={() => void perform(async () => {
                await request(`history/alarms/${record.id}/ack`, {});
                history = (await request("history/alarms")).records;
                message = "알람 기록을 확인했습니다.";
              })}>확인(ACK)</button>
            {/if}
          </p>
        {/each}
      </div>
    </details>
  {/if}
  <p role="status" class="message">{message}</p>
</section>

<style>
  .operations { padding: 8px 16px; background: var(--surface); border-bottom: 1px solid var(--border); max-height: 45vh; overflow: auto; }
  .inline { display: flex; gap: 10px; flex-wrap: wrap; align-items: end; }
  label { display: grid; gap: 4px; font-size: 12px; }
  .checkbox-label { display: flex; align-items: center; gap: 6px; padding-bottom: 6px; }
  input, select, button { font: inherit; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius); color: var(--text); background: var(--surface-2); }
  button { cursor: pointer; } button:disabled { opacity: .5; cursor: default; }
  summary { cursor: pointer; padding: 6px 0; } p { margin: 6px 0; } .message { white-space: pre-wrap; }
  .muted { color: var(--muted); font-size: 12px; } .audit { overflow-wrap: anywhere; font-size: 12px; }
  .confirmation { border-left: 3px solid var(--warn); padding: 8px; margin: 8px 0; }
  .editor-block { display: grid; gap: 8px; margin-top: 8px; }
  .editor-block h4 { margin: 4px 0 0; font-size: 13px; }
  .editor-row { border: 1px solid var(--border); border-radius: var(--radius); padding: 8px; display: grid; gap: 6px; }
  .sub-row { padding-top: 4px; border-top: 1px dashed var(--border); }
  .toml-preview { font-size: 12px; background: var(--surface-2); padding: 8px; border-radius: var(--radius); overflow: auto; max-height: 180px; }
</style>
