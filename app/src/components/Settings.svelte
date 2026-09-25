<script lang="ts">
  import { app, type Layout, type Theme } from "../lib/app.svelte";
  import { LOCALES, type Locale, systemLocale, t } from "../lib/i18n/index.svelte";

  const themes: Theme[] = ["system", "light", "dark"];
  const layouts: Layout[] = ["adaptive", "work", "code"];
  const layoutHint: Record<Layout, "layout.adaptiveHint" | "layout.workHint" | "layout.codeHint"> = {
    adaptive: "layout.adaptiveHint",
    work: "layout.workHint",
    code: "layout.codeHint",
  };
</script>

<div class="scrim" role="presentation" onclick={() => (app.overlay = null)}>
  <div class="sheet" role="dialog" aria-label={t("settings.title")} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.overlay = null)}>
    <h2>{t("settings.title")}</h2>

    <section>
      <h3>{t("settings.language")}</h3>
      <div class="choices">
        <button class:on={app.prefs.lang === "system"} onclick={() => app.setLang("system")}>{t("settings.system")} · {LOCALES[systemLocale()]}</button>
        {#each Object.entries(LOCALES) as [code, name] (code)}
          <button class:on={app.prefs.lang === code} lang={code} onclick={() => app.setLang(code as Locale)}>{name}</button>
        {/each}
      </div>
    </section>

    <section>
      <h3>{t("settings.theme")}</h3>
      <div class="choices">
        {#each themes as th (th)}
          <button class:on={app.prefs.theme === th} onclick={() => app.setTheme(th)}>{t(`theme.${th}`)}</button>
        {/each}
      </div>
    </section>

    <section>
      <h3>{t("settings.layout")}</h3>
      <div class="layouts">
        {#each layouts as l (l)}
          <button class="layout" class:on={app.prefs.layout === l} onclick={() => app.setLayout(l)}>
            <span class="lname">{t(`layout.${l}`)}</span>
            <span class="hint">{t(layoutHint[l])}</span>
          </button>
        {/each}
      </div>
    </section>

    <section class="toggles">
      <label><input type="checkbox" checked={app.prefs.vim} onchange={(e) => ((app.prefs.vim = (e.currentTarget as HTMLInputElement).checked), app.savePrefs())} /> {t("settings.vim")}</label>
      <label><input type="checkbox" checked={app.prefs.showDone} onchange={(e) => ((app.prefs.showDone = (e.currentTarget as HTMLInputElement).checked), app.savePrefs())} /> {t("settings.done")}</label>
    </section>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    background: var(--overlay);
    animation: fade-in var(--fast) var(--ease);
  }
  .sheet {
    width: min(620px, 94vw);
    max-height: 90vh;
    overflow: auto;
    padding: var(--s5) var(--s6);
    animation: pop-in var(--quick) var(--ease);
    border-radius: 16px;
    background: var(--elev);
    box-shadow: var(--shadow);
    border: 1px solid var(--line);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 18px;
  }
  h3 {
    margin: 18px 0 8px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--faint);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .choices button {
    height: 30px;
    padding: 0 var(--s3);
    border-radius: 15px;
    border: 1px solid var(--line);
  }
  .choices button,
  .layout {
    transition:
      background-color var(--fast) var(--ease),
      border-color var(--fast) var(--ease),
      color var(--fast) var(--ease),
      transform var(--fast) var(--ease);
  }
  .choices button:active,
  .layout:active {
    transform: scale(0.98);
  }
  .choices button:hover,
  .layout:hover {
    background: var(--hover);
  }
  .choices button.on,
  .layout.on {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }
  .layouts {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .layout {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
    text-align: left;
  }
  .lname {
    font-weight: 600;
  }
  .layout.on .hint {
    color: var(--accent);
    opacity: 0.8;
  }
  .toggles {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 18px;
  }
  .toggles label {
    display: flex;
    gap: 8px;
    align-items: center;
  }
</style>
