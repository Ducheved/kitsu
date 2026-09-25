<script lang="ts">
  // Settings → Projects: the workspaces list. Rename, reorder, remove (off
  // the list only; nothing on disk changes), add a folder.
  import { tick } from "svelte";
  import { app } from "../lib/app.svelte";
  import { errorText, projects } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { moved, tildify } from "../lib/repos";
  import AddProject from "./AddProject.svelte";

  let renaming = $state<string | null>(null);
  let draft = $state("");
  let nameInput: HTMLInputElement | undefined = $state();

  async function startRename(id: string, name: string) {
    renaming = id;
    draft = name;
    await tick();
    nameInput?.select();
  }

  async function saveRename() {
    const id = renaming;
    if (!id) return;
    renaming = null;
    try {
      await projects.rename(id, draft.trim() || null);
      await app.refreshProjects();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function move(id: string, delta: number) {
    const to = moved(app.projects, id, delta);
    if (to === null) return;
    try {
      await projects.move(id, to);
      await app.refreshProjects();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }

  async function remove(id: string) {
    try {
      await app.removeProject(id);
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }
</script>

<div class="list">
  {#each app.projects as p, i (p.id)}
    <div class="proj" class:here={p.id === app.repo}>
      <div class="text">
        {#if renaming === p.id}
          <input
            bind:this={nameInput}
            class="field rename"
            bind:value={draft}
            title={t("projects.renameHint")}
            onkeydown={(e) => {
              if (e.key === "Enter") void saveRename();
              else if (e.key === "Escape") {
                e.stopPropagation();
                renaming = null;
              }
            }}
            onblur={() => void saveRename()}
          />
        {:else}
          <span class="line">
            <button class="pname" title={t("projects.rename")} onclick={() => void startRename(p.id, p.name)}>{p.name}</button>
            {#if p.id === app.repo}<span class="open">{t("projects.open")}</span>{/if}
            {#if p.missing}<span class="tone-bad small">{t("project.gone")}</span>{:else if !p.trusted}<span class="tone-warn small">{t("sb.untrusted")}</span>{/if}
          </span>
        {/if}
        <span class="root mono">{tildify(p.root)}</span>
      </div>
      <div class="tools">
        <button class="icon" title={t("projects.up")} aria-label={t("projects.up")} disabled={i === 0} onclick={() => void move(p.id, -1)}>↑</button>
        <button class="icon" title={t("projects.down")} aria-label={t("projects.down")} disabled={i === app.projects.length - 1} onclick={() => void move(p.id, 1)}>↓</button>
        <button class="btn quiet mini" onclick={() => void startRename(p.id, p.name)}>{t("projects.rename")}</button>
        <button class="btn quiet mini danger" title={t("projects.removeTitle")} onclick={() => void remove(p.id)}>{t("projects.remove")}</button>
      </div>
    </div>
  {/each}
</div>
<AddProject focus={app.addingProject} onadded={() => (app.addingProject = false)} />
<p class="hint foot">{t("projects.hint")}</p>

<style>
  .list {
    display: flex;
    flex-direction: column;
    margin-bottom: var(--s3);
  }
  .proj {
    display: flex;
    align-items: center;
    gap: var(--s3);
    padding: var(--s2) 0;
    border-bottom: 1px solid var(--line);
  }
  .text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: var(--s2);
  }
  .pname {
    font-weight: 500;
    text-align: left;
  }
  .pname:hover {
    color: var(--accent);
  }
  .here .pname {
    font-weight: 650;
  }
  .open {
    padding: 0 7px;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
  }
  .small {
    font-size: 11.5px;
  }
  .root {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    font-size: 11.5px;
  }
  .rename {
    padding: 3px var(--s2);
    margin: -3px 0 1px calc(-1 * var(--s2));
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
    opacity: 0.55;
    transition: opacity var(--fast) var(--ease);
  }
  .proj:hover .tools,
  .tools:focus-within {
    opacity: 1;
  }
  .icon {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    color: var(--muted);
    transition:
      background-color var(--fast) var(--ease),
      color var(--fast) var(--ease);
  }
  .icon:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .icon:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .mini {
    height: 26px;
    padding: 0 9px;
    font-size: 12.5px;
  }
  .danger:hover {
    color: var(--bad);
  }
  .foot {
    margin: var(--s2) 0 0;
    line-height: 1.5;
  }
</style>
