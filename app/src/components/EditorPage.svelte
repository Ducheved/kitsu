<script lang="ts">
  import { EditorState } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { onDestroy } from "svelte";
  import { app } from "../lib/app.svelte";
  import { api, errorKind, errorText } from "../lib/api";
  import { base, setExTarget } from "../lib/editor";

  let { path }: { path: string } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let version: string | null = null;
  let saved = $state("");
  let dirty = $state(false);
  let error = $state<string | null>(null);
  let conflict = $state(false);

  async function load(p: string) {
    error = null;
    conflict = false;
    try {
      const f = await api.readFile(p);
      version = f.version;
      saved = f.text;
      dirty = false;
      view?.destroy();
      if (!host) return;
      view = new EditorView({
        parent: host,
        state: EditorState.create({
          doc: f.text,
          extensions: [
            base(p, { vim: app.prefs.vim }),
            EditorView.updateListener.of((u) => {
              if (u.docChanged) dirty = u.state.doc.toString() !== saved;
            }),
          ],
        }),
      });
      view.focus();
    } catch (e) {
      error = errorText(e);
    }
  }

  export async function save() {
    if (!view) return;
    const text = view.state.doc.toString();
    try {
      version = await api.writeFile(path, text, version);
      saved = text;
      dirty = false;
      conflict = false;
      app.notify(`Saved ${path}`, "ok");
    } catch (e) {
      if (errorKind(e) === "conflict") conflict = true;
      app.notify(errorText(e), "bad");
    }
  }

  function close() {
    if (dirty && !confirm(`${path} has unsaved changes. Close anyway?`)) return;
    app.goBack();
  }

  $effect(() => {
    const p = path;
    const _vim = app.prefs.vim;
    void _vim;
    setExTarget({ save, close, open: (np) => app.go({ kind: "file", path: np }) });
    void load(p);
  });

  onDestroy(() => {
    view?.destroy();
    setExTarget(null);
  });
</script>

<div class="editor-page">
  <header>
    <button class="btn quiet" onclick={close}>←</button>
    <span class="mono path">{path}</span>
    {#if dirty}<span class="pill warn">unsaved</span>{/if}
    {#if conflict}<span class="pill bad">changed on disk</span><button class="btn quiet" onclick={() => load(path)}>Reload</button>{/if}
    <span class="spacer"></span>
    <span class="hint">{app.prefs.vim ? "vim · :w saves · :q closes" : "⌘S saves"}</span>
    <button class="btn" disabled={!dirty} onclick={save}>Save</button>
  </header>
  {#if error}<div class="err tone-bad">{error}</div>{/if}
  <div
    class="host"
    bind:this={host}
    role="presentation"
    onkeydown={(e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        void save();
      }
    }}
  ></div>
</div>

<style>
  .editor-page {
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
  .err {
    padding: 12px 16px;
  }
</style>
