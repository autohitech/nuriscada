<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright NuriScada contributors -->
<script lang="ts">
  import AlarmPanel from "./components/AlarmPanel.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import TagTable from "./components/TagTable.svelte";
  import { Store, connect, pollStatus } from "./lib/store.svelte";
  import { ko } from "./lib/strings";
  import OperationsPanel, { type SessionDoc } from "./components/OperationsPanel.svelte";

  const THEMES = ["dark", "light", "contrast"] as const;
  type Theme = (typeof THEMES)[number];
  const STORAGE_KEY = "nuriscada.theme";

  const store = new Store();
  let session = $state<SessionDoc | null>(null);
  const canRead = $derived(!!session && (!session.authentication_required || !!session.user));

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
    <TagTable {store} />
    <AlarmPanel {store} />
  </main>{:else}<main class="welcome"><p>로그인하면 실시간 값과 알람을 볼 수 있습니다.</p></main>{/if}
  <footer>{ko.safetyNotice}</footer>
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
