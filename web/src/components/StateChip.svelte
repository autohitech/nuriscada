<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import type { Severity } from "../lib/format";

  let { severity, label }: { severity: Severity; label: string } = $props();
</script>

<!-- 상태는 색만으로 전달하지 않는다: 아이콘 모양과 문구를 함께 쓴다. -->
<span class="chip {severity}">
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
    {#if severity === "critical"}
      <path d="M8 1 15 8 8 15 1 8Z" fill="currentColor" />
      <path d="M8 4.5v4.2M8 10.4v1.1" stroke="var(--critical-bg)" stroke-width="1.6" stroke-linecap="round" />
    {:else if severity === "warn"}
      <path d="M8 1.5 15 14H1Z" fill="currentColor" />
      <path d="M8 6v4M8 11.4v.8" stroke="var(--warn-bg)" stroke-width="1.6" stroke-linecap="round" />
    {:else if severity === "info"}
      <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.6" />
      <path d="M8 7v4M8 4.8v.8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
    {:else if severity === "bad"}
      <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-dasharray="2.5 2" />
      <path d="M5.5 5.5 10.5 10.5M10.5 5.5 5.5 10.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    {:else}
      <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.6" />
      <path d="m5 8.2 2.2 2.2L11 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
    {/if}
  </svg>
  <span>{label}</span>
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid currentColor;
    font-size: 12px;
    white-space: nowrap;
  }
  .normal {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
    background: var(--warn-bg);
  }
  .critical {
    color: var(--critical);
    background: var(--critical-bg);
    font-weight: 600;
  }
  .info {
    color: var(--info);
    background: var(--info-bg);
  }
  .bad {
    color: var(--bad);
    background: var(--bad-bg);
    border-style: dashed;
  }
</style>
