<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import { formatTime, formatValue, severityOfEvent, severityOfState, shouldBeep } from "../lib/format";
  import type { AlarmSoundRule } from "../lib/format";
  import type { Store } from "../lib/store.svelte";
  import { stateLabel, eventLabel, m } from "../lib/i18n.svelte";
  import StateChip from "./StateChip.svelte";

  let { store }: { store: Store } = $props();

  /** 활성 알람은 화면에 이만큼만 그리고 나머지는 건수로 알린다(알람 폭주 때 화면이 느려지지 않게). */
  const ACTIVE_VISIBLE = 100;

  const active = $derived(
    store.activeNames
      .slice(0, ACTIVE_VISIBLE)
      .map((n) => store.indexOf(n))
      .filter((i): i is number => i !== undefined)
      .map((i) => store.tags[i])
      .filter((t) => t !== undefined),
  );
  const recent = $derived([...store.recent].reverse().slice(0, 100));
  const announcement = $derived(
    store.latestAlarm
      ? `${store.latestAlarm.tag}: ${eventLabel(store.latestAlarm.event)}`
      : "",
  );
  const SOUND_KEY = "nuriscada.alarm-sound";
  function loadSoundRule(): AlarmSoundRule {
    try {
      const saved = localStorage.getItem(SOUND_KEY);
      if (saved === "critical" || saved === "warn" || saved === "all") return saved;
    } catch {
      // 저장소를 쓸 수 없으면 끔으로 둔다.
    }
    return "off";
  }
  let soundRule = $state<AlarmSoundRule>(loadSoundRule());
  let audio: AudioContext | null = null;
  let lastBeepedSeq = 0;
  function beep(critical: boolean): void {
    try {
      audio ??= new AudioContext();
      if (audio.state === "suspended") void audio.resume();
      const osc = audio.createOscillator();
      const gain = audio.createGain();
      osc.frequency.value = critical ? 880 : 440;
      gain.gain.value = 0.15;
      osc.connect(gain);
      gain.connect(audio.destination);
      osc.start();
      osc.stop(audio.currentTime + (critical ? 0.4 : 0.2));
    } catch {
      // 소리를 낼 수 없으면 조용히 넘어간다.
    }
  }
  $effect(() => {
    try {
      localStorage.setItem(SOUND_KEY, soundRule);
    } catch {
      // 저장 실패는 무시한다.
    }
    const latest = store.latestAlarm;
    if (latest && latest.seq > lastBeepedSeq) {
      lastBeepedSeq = latest.seq;
      const severity = severityOfEvent(latest.event);
      if (shouldBeep(soundRule, severity)) beep(severity === "critical");
    }
  });
</script>

<aside class="panel" aria-label={m.alarms}>
  <!-- 새 알람을 화면 낭독기에 알린다. -->
  <p class="sr-only" role="status" aria-live="assertive">{announcement}</p>

  <section>
    <h2>{m.activeAlarms} <span class="count">{store.activeNames.length}</span></h2>
    <p class="muted">
      <label>{m.alarmSound}
        <select bind:value={soundRule}>
          <option value="off">{m.soundOff}</option>
          <option value="critical">{m.soundCritical}</option>
          <option value="warn">{m.soundWarn}</option>
          <option value="all">{m.soundAll}</option>
        </select>
      </label>
    </p>
    {#if store.activeNames.length === 0}
      <p class="empty">{m.noAlarms}</p>
    {:else}
      <ul>
        {#each active as t (t.name)}
          <li class={severityOfState(t.alarm_state)}>
            <span class="name">{t.name}</span>
            <StateChip
              severity={severityOfState(t.alarm_state)}
              label={t.alarm_state ? stateLabel(t.alarm_state) : ""}
            />
            <span class="val">{formatValue(t.value)} {t.unit_text}</span>
          </li>
        {/each}
      </ul>
      {#if store.activeNames.length > ACTIVE_VISIBLE}
        <p class="empty">{m.moreAlarms(store.activeNames.length - ACTIVE_VISIBLE)}</p>
      {/if}
    {/if}
  </section>

  <section>
    <h2>{m.recentAlarms}</h2>
    {#if recent.length === 0}
      <p class="empty">{m.noRecent}</p>
    {:else}
      <ul class="log">
        {#each recent as r (r.seq)}
          <li>
            <time>{formatTime(r.time_ms)}</time>
            <span class="name">{r.tag}</span>
            <StateChip severity={severityOfEvent(r.event)} label={eventLabel(r.event)} />
            <span class="val">{formatValue(r.value)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</aside>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    min-height: 0;
    overflow: auto;
    padding: 12px;
    background: var(--surface);
    border-left: 1px solid var(--border);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 13px;
    color: var(--muted);
    font-weight: 600;
  }
  .count {
    margin-left: 6px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
    padding: 6px 8px;
    border-radius: var(--radius);
    background: var(--surface-2);
    border-left: 3px solid transparent;
  }
  li.critical {
    border-left-color: var(--critical);
    background: var(--critical-bg);
  }
  li.warn {
    border-left-color: var(--warn);
    background: var(--warn-bg);
  }
  .name {
    font-weight: 600;
    margin-right: auto;
  }
  .val,
  time {
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .log li {
    background: transparent;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
  }
  .empty {
    margin: 0;
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
</style>
