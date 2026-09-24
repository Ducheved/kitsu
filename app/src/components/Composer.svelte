<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";

  let title = $state("");
  let body = $state("");
  let scope = $state("");
  let busy = $state(false);
  let titleInput: HTMLInputElement | undefined = $state();

  onMount(() => titleInput?.focus());

  async function create() {
    if (!title.trim() || busy) return;
    busy = true;
    try {
      const { id } = await api.newEntity("task", title, {
        body,
        scope: scope
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      app.overlay = null;
      await app.refresh();
      app.openTask(id);
    } catch (e) {
      app.notify(errorText(e), "bad");
    } finally {
      busy = false;
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") app.overlay = null;
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void create();
  }
</script>

<div class="scrim" role="presentation" onclick={() => (app.overlay = null)}>
  <div class="sheet" role="dialog" aria-label="New task" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={key}>
    <input bind:this={titleInput} class="title" placeholder="What needs to happen?" bind:value={title} onkeydown={(e) => e.key === "Enter" && !e.shiftKey && (e.preventDefault(), create())} />
    <textarea class="body" rows="5" placeholder="Why, and what done looks like. The agent reads this first." bind:value={body}></textarea>
    <label class="scope">
      <span class="hint">Scope</span>
      <input class="field mono" placeholder="src/payments/**, api/charge.py  (rules that cover these paths get attached)" bind:value={scope} />
    </label>
    <div class="foot">
      <span class="hint">Saved as a file in .kitsu/tasks/. Add checks there or in kitsu.toml.</span>
      <button class="btn primary" disabled={!title.trim() || busy} onclick={create}>Create <kbd>⏎</kbd></button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 14vh;
    background: var(--overlay);
  }
  .sheet {
    display: flex;
    flex-direction: column;
    width: min(640px, 92vw);
    border-radius: 16px;
    background: var(--elev);
    box-shadow: var(--shadow);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .title {
    padding: 20px 22px 8px;
    border: none;
    background: transparent;
    font-size: 20px;
    font-weight: 600;
    outline: none;
  }
  .body {
    padding: 4px 22px 14px;
    border: none;
    background: transparent;
    resize: none;
    outline: none;
    color: var(--text);
  }
  .scope {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 22px 14px;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 22px;
    border-top: 1px solid var(--line);
    background: var(--rail);
  }
</style>
