<script lang="ts">
  import { unifiedMergeView } from "@codemirror/merge";
  import { EditorState } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { onDestroy } from "svelte";
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { base } from "../lib/editor";
  import type { FileDiff } from "../lib/types";

  let { run, path }: { run: string; path: string } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let diff = $state<FileDiff | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const r = run;
    const p = path;
    api
      .fileDiff(r, p)
      .then((d) => {
        diff = d;
        view?.destroy();
        if (!host) return;
        view = new EditorView({
          parent: host,
          state: EditorState.create({
            doc: d.new ?? "",
            extensions: [
              base(p, { vim: app.prefs.vim, readOnly: true }),
              EditorState.readOnly.of(true),
              unifiedMergeView({ original: d.old ?? "", mergeControls: false, gutter: true }),
            ],
          }),
        });
      })
      .catch((e) => (error = errorText(e)));
  });

  onDestroy(() => view?.destroy());
</script>

<div class="diff-page">
  <header>
    <button class="btn quiet" onclick={() => app.goBack()}>←</button>
    <span class="mono path">{path}</span>
    {#if diff?.old === null}<span class="pill ok">new file</span>{/if}
    {#if diff?.new === null}<span class="pill bad">deleted</span>{/if}
    {#if diff?.protected}<span class="pill warn">rules / protected</span>{/if}
    <span class="spacer"></span>
    <span class="hint mono">run {run}</span>
  </header>
  {#if error}<div class="err tone-bad">{error}</div>{/if}
  <div class="host" bind:this={host}></div>
</div>

<style>
  .diff-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }
  .path {
    font-size: 13px;
  }
  .spacer {
    flex: 1;
  }
  .host {
    flex: 1;
    min-height: 0;
  }
  .host :global(.cm-editor) {
    height: 100%;
  }
  .host :global(.cm-changedLine) {
    background: var(--ok-soft) !important;
  }
  .host :global(.cm-deletedChunk) {
    background: var(--bad-soft) !important;
  }
  .err {
    padding: 12px 16px;
  }
</style>
