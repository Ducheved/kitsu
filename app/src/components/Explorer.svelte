<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { api } from "../lib/api";
  import { buffers } from "../lib/buffers.svelte";
  import { t } from "../lib/i18n/index.svelte";

  interface Node {
    name: string;
    path: string;
    dir: boolean;
    children: Node[];
  }

  let files = $state<string[]>([]);
  let open = $state<Record<string, boolean>>({});
  let filter = $state("");
  let cursor = $state<string | null>(null);
  let tree: HTMLDivElement | undefined = $state();

  // Refresh the file list when something changed on disk (agents accepted,
  // files created). The list comes from git: tracked plus untracked files
  // that aren't ignored.
  $effect(() => {
    void app.tick;
    api.listFiles().then((f) => (files = f)).catch(() => {});
  });

  function build(paths: string[]): Node[] {
    const root: Node = { name: "", path: "", dir: true, children: [] };
    for (const p of paths) {
      let node = root;
      const parts = p.split("/");
      parts.forEach((part, i) => {
        const path = parts.slice(0, i + 1).join("/");
        const dir = i < parts.length - 1;
        let child = node.children.find((c) => c.name === part && c.dir === dir);
        if (!child) {
          child = { name: part, path, dir, children: [] };
          node.children.push(child);
        }
        node = child;
      });
    }
    const sort = (n: Node) => {
      n.children.sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name));
      n.children.forEach(sort);
    };
    sort(root);
    return root.children;
  }

  const q = $derived(filter.trim().toLowerCase());
  const nodes = $derived(build(q ? files.filter((f) => f.toLowerCase().includes(q)) : files));

  // Flattened visible rows, so j/k move through what's on screen.
  const rows = $derived.by(() => {
    const out: { node: Node; depth: number }[] = [];
    const walk = (list: Node[], depth: number) => {
      for (const n of list) {
        out.push({ node: n, depth });
        if (n.dir && (open[n.path] || q)) walk(n.children, depth + 1);
      }
    };
    walk(nodes, 0);
    return out;
  });

  function activate(n: Node) {
    cursor = n.path;
    if (n.dir) open[n.path] = !open[n.path];
    else app.go({ kind: "file", path: n.path });
  }

  function key(e: KeyboardEvent) {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const i = Math.max(0, rows.findIndex((r) => r.node.path === cursor));
    const at = rows[i]?.node;
    const move = (d: number) => {
      const r = rows[Math.min(rows.length - 1, Math.max(0, i + d))];
      if (r) {
        cursor = r.node.path;
        tree?.querySelector(`[data-path="${CSS.escape(r.node.path)}"]`)?.scrollIntoView({ block: "nearest" });
      }
    };
    switch (e.key) {
      case "j":
      case "ArrowDown":
        move(1);
        break;
      case "k":
      case "ArrowUp":
        move(-1);
        break;
      case "l":
      case "ArrowRight":
        if (at?.dir) open[at.path] = true;
        else if (at) activate(at);
        break;
      case "h":
      case "ArrowLeft":
        if (at?.dir && open[at.path]) open[at.path] = false;
        else if (at) {
          const parent = at.path.split("/").slice(0, -1).join("/");
          if (parent) {
            cursor = parent;
            open[parent] = false;
          }
        }
        break;
      case "Enter":
      case "o":
        if (at) activate(at);
        break;
      case "/":
        (tree?.parentElement?.querySelector("input") as HTMLInputElement | null)?.focus();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  export function focus() {
    if (!cursor && rows[0]) cursor = rows[0].node.path;
    tree?.focus();
  }
</script>

<aside class="explorer">
  <header>
    <span class="title">{t("explorer.title")}</span>
    <span class="hint mono">{app.overview?.repo.name ?? ""}</span>
  </header>
  <div class="filter">
    <input class="field" placeholder={t("explorer.filter")} bind:value={filter} onkeydown={(e) => {
      if (e.key === "Escape" || e.key === "Enter") {
        if (e.key === "Escape") filter = "";
        (e.currentTarget as HTMLInputElement).blur();
        tree?.focus();
      }
    }} />
  </div>
  <div class="tree scroll" bind:this={tree} tabindex="0" role="tree" aria-label={t("explorer.title")} onkeydown={key}>
    {#each rows as { node, depth } (node.path + node.dir)}
      <button
        class="row"
        class:cursor={cursor === node.path}
        class:active={!node.dir && buffers.active === node.path}
        style="padding-left: {10 + depth * 14}px"
        data-path={node.path}
        role="treeitem"
        aria-selected={cursor === node.path}
        aria-expanded={node.dir ? !!open[node.path] || !!q : undefined}
        tabindex="-1"
        onclick={() => activate(node)}
      >
        <span class="chev">{node.dir ? (open[node.path] || q ? "▾" : "▸") : ""}</span>
        <span class="n" class:dir={node.dir}>{node.name}</span>
        {#if !node.dir && buffers.open.some((b) => b.path === node.path && b.dirty)}<span class="dot">●</span>{/if}
      </button>
    {:else}
      <div class="hint pad">{t("explorer.empty")}</div>
    {/each}
  </div>
</aside>

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--rail);
    border-right: 1px solid var(--line);
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    padding: 14px 14px 8px;
  }
  .title {
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--faint);
  }
  .filter {
    padding: 0 10px 8px;
  }
  .filter .field {
    padding: 6px 10px;
    font-size: 12.5px;
  }
  .tree {
    flex: 1;
    min-height: 0;
    padding: 2px 4px 12px;
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    height: 24px;
    padding-right: 8px;
    border-radius: 6px;
    text-align: left;
    font-size: 13px;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    color: var(--accent);
  }
  .tree:focus-within .row.cursor,
  .tree:focus .row.cursor {
    background: var(--active);
  }
  .chev {
    width: 12px;
    flex: none;
    color: var(--faint);
    font-size: 10px;
  }
  .n {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dir {
    font-weight: 500;
  }
  .dot {
    margin-left: auto;
    color: var(--accent);
    font-size: 9px;
  }
  .pad {
    padding: 10px 12px;
  }
</style>
