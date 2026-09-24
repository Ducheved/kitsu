<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { buffers } from "../lib/buffers.svelte";
  import { LOCALES, i18n, t } from "../lib/i18n/index.svelte";

  const ov = $derived(app.overview);
  const needs = $derived((ov?.tasks ?? []).filter((x) => x.attention === "needs_you").length);
  const running = $derived((ov?.tasks ?? []).filter((x) => x.attention === "working").length);
</script>

<footer class="bar">
  <div class="modes" role="radiogroup" aria-label={t("settings.layout")}>
    <button role="radio" aria-checked={app.mode === "work"} class:on={app.mode === "work"} onclick={() => app.setMode("work")} title="⌘1">{t("layout.work")}</button>
    <button role="radio" aria-checked={app.mode === "code"} class:on={app.mode === "code"} onclick={() => app.setMode("code")} title="⌘2">{t("layout.code")}</button>
  </div>
  {#if ov?.repo.branch}<span class="item mono">⎇ {ov.repo.branch}</span>{/if}
  {#if ov && !ov.repo.trusted}<span class="item tone-warn">{t("sb.untrusted")}</span>{/if}
  {#if needs}
    <button class="item tone-warn" onclick={() => app.setMode("work")}>● {t("sb.needsYou", { n: needs })}</button>
  {/if}
  {#if running}<span class="item tone-work">◌ {t("sb.running", { n: running })}</span>{/if}
  <span class="spacer"></span>
  {#if app.mode === "code" && buffers.current}<span class="item mono">{buffers.current.path}</span>{/if}
  {#if app.prefs.vim}<span class="item mono">VIM</span>{/if}
  <button class="item" onclick={() => (app.overlay = "settings")} title={t("cmd.settings")}>{LOCALES[i18n.locale]}</button>
</footer>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 26px;
    padding: 0 10px;
    border-top: 1px solid var(--line);
    background: var(--rail);
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
  }
  .modes {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 7px;
    background: var(--hover);
  }
  .modes button {
    height: 18px;
    padding: 0 8px;
    border-radius: 5px;
    font-size: 11.5px;
  }
  .modes button.on {
    background: var(--elev);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--line);
  }
  .item {
    font-size: 12px;
  }
  button.item:hover {
    color: var(--text);
  }
  .mono {
    font-size: 11.5px;
  }
  .spacer {
    flex: 1;
  }
</style>
