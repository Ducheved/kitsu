<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../lib/app.svelte";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { statusText } from "../lib/status";
  import type { Command } from "../lib/types";

  let { commands, files = false }: { commands: Command[]; files?: boolean } = $props();

  let query = $state("");
  let index = $state(0);
  let fileList = $state<string[]>([]);
  let input: HTMLInputElement | undefined = $state();

  onMount(() => {
    input?.focus();
    api.listFiles().then((f) => (fileList = f)).catch(() => {});
  });

  // Subsequence match with a small bonus for word starts and contiguity.
  function score(text: string, q: string): number {
    if (!q) return 1;
    const hay = text.toLowerCase();
    let ti = 0;
    let s = 0;
    let run = 0;
    for (const ch of q.toLowerCase()) {
      const at = hay.indexOf(ch, ti);
      if (at < 0) return 0;
      run = at === ti ? run + 1 : 0;
      s += 1 + run * 2 + (at === 0 || /[\s/_.-]/.test(hay[at - 1] ?? "") ? 3 : 0);
      ti = at + 1;
    }
    return s - hay.length * 0.01;
  }

  const items = $derived.by(() => {
    const q = query.trim();
    const pool: Command[] = files
      ? []
      : [
          ...commands,
          ...(app.overview?.tasks ?? []).map((task) => ({ id: `task:${task.id}`, title: task.title, hint: statusText(task), group: "tasks" as const, run: () => app.openTask(task.id) })),
        ];
    const fileItems: Command[] = fileList.map((f) => ({ id: `file:${f}`, title: f, group: "files" as const, run: () => app.go({ kind: "file", path: f }) }));
    const all = files || q ? [...pool, ...fileItems] : pool;
    return all
      .map((c) => ({ c, s: score(c.title, q) + (c.group === "commands" ? 0.5 : 0) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 50)
      .map((x) => x.c);
  });

  $effect(() => {
    void query;
    index = 0;
  });

  function pick(c: Command | undefined) {
    if (!c) return;
    app.overlay = null;
    c.run();
  }

  function key(e: KeyboardEvent) {
    const down = e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j"));
    const up = e.key === "ArrowUp" || (e.ctrlKey && (e.key === "p" || e.key === "k"));
    if (down) {
      index = Math.min(index + 1, items.length - 1);
      e.preventDefault();
    } else if (up) {
      index = Math.max(index - 1, 0);
      e.preventDefault();
    } else if (e.key === "Enter") {
      pick(items[index]);
      e.preventDefault();
    } else if (e.key === "Escape") {
      app.overlay = null;
      e.preventDefault();
    }
  }
</script>

<div class="scrim" role="presentation" onclick={() => (app.overlay = null)}>
  <div class="palette" role="dialog" aria-label={t("group.commands")} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <input bind:this={input} class="q" placeholder={files ? t("palette.files") : t("palette.placeholder")} bind:value={query} onkeydown={key} />
    <div class="results scroll" role="listbox">
      {#each items as c, i (c.id)}
        {#if i === 0 || items[i - 1]?.group !== c.group}<div class="group">{t(`group.${c.group}`)}</div>{/if}
        <button class="item" class:on={i === index} role="option" aria-selected={i === index} onmouseenter={() => (index = i)} onclick={() => pick(c)}>
          <span class="title" class:mono={c.group === "files"}>{c.title}</span>
          {#if c.hint}<span class="hint">{c.hint}</span>{/if}
        </button>
      {:else}
        <div class="none hint">{t("palette.none")}</div>
      {/each}
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
    padding-top: 12vh;
    background: var(--overlay);
  }
  .palette {
    width: min(620px, 92vw);
    border-radius: 14px;
    background: var(--elev);
    box-shadow: var(--shadow);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .q {
    width: 100%;
    padding: 16px 18px;
    border: none;
    border-bottom: 1px solid var(--line);
    background: transparent;
    font-size: 16px;
    outline: none;
  }
  .results {
    max-height: 50vh;
    padding: 6px;
  }
  .group {
    padding: 8px 12px 4px;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--faint);
  }
  .item {
    display: flex;
    align-items: baseline;
    gap: 12px;
    width: 100%;
    padding: 8px 12px;
    border-radius: 8px;
    text-align: left;
  }
  .item.on {
    background: var(--active);
  }
  .title {
    flex: none;
    max-width: 70%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item .hint {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    padding: 16px;
  }
</style>
