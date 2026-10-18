<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import AlarmPanel from "./components/AlarmPanel.svelte";
  import GraphicScreen from "./components/GraphicScreen.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import TagTable from "./components/TagTable.svelte";
  import { Store, connect, pollStatus } from "./lib/store.svelte";
  import { initLocale, m } from "./lib/i18n.svelte";
  import OperationsPanel, { type SessionDoc } from "./components/OperationsPanel.svelte";

  const THEMES = ["dark", "light", "contrast"] as const;
  type Theme = (typeof THEMES)[number];
  const STORAGE_KEY = "nuriscada.theme";

  initLocale();
  const store = new Store();
  let session = $state<SessionDoc | null>(null);
  let viewMode = $state<"split" | "graphic" | "table">("split");
  const canRead = $derived(!!session && (!session.authentication_required || !!session.user));
  const canEditGraphic = $derived(
    !!session && !!session.local_management && session.user?.role === "admin"
  );

  function loadTheme(): Theme {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved && (THEMES as readonly string[]).includes(saved)) return saved as Theme;
    } catch {
      // 저장소를 쓸 수 없으면 기본(다크)을 쓴다.
    }
    return "dark";
  }

  let theme = $state<Theme>(loadTheme());

  $effect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // 저장하지 못해도 화면은 정상이다.
    }
  });

  function nextTheme() {
    theme = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length] ?? "dark";
  }

  $effect(() => {
    if (!canRead) return;
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const stopStream = connect({ url: `${scheme}://${location.host}/api/v1/stream`, store });
    const stopStatus = pollStatus(store);
    return () => {
      stopStream();
      stopStatus();
    };
  });
</script>

<div class="app">
  {#if canRead}<StatusBar {store} {theme} onTheme={nextTheme} />{:else}<header class="welcome">NuriScada</header>{/if}
  <OperationsPanel onSession={(s) => session = s} />
  {#if canRead}<main>
    <div class="workspace-col">
      <div class="view-mode-bar" role="toolbar" aria-label={m.viewModeAria}>
        <button type="button" class:active={viewMode === "split"} onclick={() => viewMode = "split"}>{m.viewSplit}</button>
        <button type="button" class:active={viewMode === "graphic"} onclick={() => viewMode = "graphic"}>{m.viewGraphic}</button>
        <button type="button" class:active={viewMode === "table"} onclick={() => viewMode = "table"}>{m.viewTable}</button>
      </div>
      {#if viewMode !== "table"}
        <GraphicScreen {store} canEdit={canEditGraphic} />
      {/if}
      {#if viewMode !== "graphic"}
        <TagTable {store} />
      {/if}
    </div>
    <AlarmPanel {store} />
  </main>{:else}<main class="welcome"><p>{m.loginToSee}</p></main>{/if}
  <footer>{m.safetyNotice}</footer>
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    height: 100%;
  }
  main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
    min-height: 0;
  }
  .workspace-col {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .view-mode-bar {
    display: flex;
    gap: 6px;
    padding: 6px 14px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .view-mode-bar button {
    font: inherit;
    font-size: 12px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .view-mode-bar button.active {
    border-color: var(--accent, #4ea1ff);
    font-weight: 600;
  }
  .welcome { display: block; padding: 24px; color: var(--muted); }
  footer {
    padding: 6px 16px;
    color: var(--muted);
    font-size: 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  @media (max-width: 900px) {
    main {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr) minmax(0, 40%);
    }
  }
</style>
