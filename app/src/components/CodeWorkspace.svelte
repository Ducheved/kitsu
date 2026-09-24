<script lang="ts">
  import { EditorView } from "@codemirror/view";
  import { onDestroy } from "svelte";
  import { app } from "../lib/app.svelte";
  import { buffers } from "../lib/buffers.svelte";
  import { highlightExt, highlightSlot, setExTarget, vimExt, vimSlot } from "../lib/editor";
  import { t } from "../lib/i18n/index.svelte";

  let { path }: { path?: string } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let shownPath: string | null = null;

  // Open whatever the route asks for.
  $effect(() => {
    if (path) void buffers.show(path);
  });

  // One view, many states: swap the state when the active tab changes.
  $effect(() => {
    const b = buffers.current;
    if (!host) return;
    if (!b) {
      view?.destroy();
      view = null;
      shownPath = null;
      return;
    }
    if (!view) {
      view = new EditorView({
        parent: host,
        state: b.state,
        dispatch: (tr, v) => {
          v.update([tr]);
          if (shownPath) buffers.update(shownPath, v.state);
        },
      });
    } else if (shownPath !== b.path) {
      view.setState(b.state);
      // A background tab may have been created with older preferences.
      view.dispatch({ effects: [vimSlot.reconfigure(vimExt(app.prefs.vim)), highlightSlot.reconfigure(highlightExt(app.dark))] });
    }
    shownPath = b.path;
    view.focus();
  });

  // Preferences apply to the open tab now and to the others when shown.
  $effect(() => {
    const vimOn = app.prefs.vim;
    const dark = app.dark;
    if (!view) return;
    view.dispatch({ effects: [vimSlot.reconfigure(vimExt(vimOn)), highlightSlot.reconfigure(highlightExt(dark))] });
  });

  setExTarget({
    save: () => void buffers.save(),
    close: () => buffers.close(),
    open: (p) => app.go({ kind: "file", path: p }),
    next: () => buffers.cycle(1),
    prev: () => buffers.cycle(-1),
  });

  onDestroy(() => {
    view?.destroy();
    setExTarget(null);
  });
</script>

<div class="workspace">
  <div class="tabs" role="tablist">
    {#each buffers.open as b (b.path)}
      <div class="tab" class:on={b.path === buffers.active} role="tab" aria-selected={b.path === buffers.active}>
        <button class="name mono" onclick={() => (buffers.active = b.path)} title={b.path}>
          {b.path.split("/").pop()}
          {#if b.dirty}<span class="dot" aria-label={t("editor.unsaved")}>●</span>{/if}
        </button>
        <button class="x" aria-label="close" onclick={() => buffers.close(b.path)}>×</button>
      </div>
    {/each}
    <span class="spacer"></span>
    {#if buffers.current}
      {#if buffers.current.conflict}
        <span class="pill bad">{t("editor.changedOnDisk")}</span>
        <button class="btn quiet mini" onclick={() => buffers.current && buffers.reload(buffers.current.path)}>{t("editor.reload")}</button>
      {/if}
      <span class="hint">{app.prefs.vim ? t("editor.vimHint") : t("editor.saveHint")}</span>
    {/if}
  </div>
  {#if buffers.current}<div class="crumb mono hint">{buffers.current.path}</div>{/if}
  <div
    class="host"
    bind:this={host}
    role="presentation"
    onkeydown={(e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        void buffers.save();
      }
    }}
  ></div>
  {#if !buffers.current}
    <div class="empty">
      <p>{t("editor.noFiles")}</p>
      <p class="hint">{t("editor.noFilesHint", { key: "⌘P" })}</p>
    </div>
  {/if}
</div>

<style>
  .workspace {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 36px;
    padding: 0 8px;
    border-bottom: 1px solid var(--line);
    background: var(--rail);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    height: 28px;
    border-radius: 7px;
    color: var(--muted);
  }
  .tab.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--line);
  }
  .name {
    padding: 0 4px 0 10px;
    font-size: 12.5px;
    white-space: nowrap;
  }
  .dot {
    margin-left: 4px;
    color: var(--accent);
    font-size: 9px;
  }
  .x {
    width: 20px;
    color: var(--faint);
    border-radius: 5px;
  }
  .x:hover {
    color: var(--text);
    background: var(--hover);
  }
  .spacer {
    flex: 1;
  }
  .mini {
    height: 24px;
    padding: 0 8px;
  }
  .crumb {
    padding: 4px 16px;
    font-size: 11.5px;
    border-bottom: 1px solid var(--line);
  }
  .host {
    flex: 1;
    min-height: 0;
  }
  .host :global(.cm-editor) {
    height: 100%;
  }
  .empty {
    position: absolute;
    inset: 36px 0 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--muted);
  }
  .empty p {
    margin: 2px;
  }
</style>
