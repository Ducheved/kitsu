<script lang="ts">
  // "Add a folder": a path field. The backend refuses what isn't a git
  // repository's top level, or is already listed, and says why.
  import { onMount } from "svelte";
  import { app } from "../lib/app.svelte";
  import { errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";

  let { focus = false, onadded }: { focus?: boolean; onadded?: () => void } = $props();

  let path = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let input: HTMLInputElement | undefined = $state();

  onMount(() => {
    if (focus) input?.focus();
  });

  async function add(e?: Event) {
    e?.preventDefault();
    if (!path.trim() || busy) return;
    busy = true;
    try {
      await app.addProject(path.trim());
      path = "";
      error = null;
      onadded?.();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<form class="add" onsubmit={add}>
  <input
    bind:this={input}
    class="field mono"
    placeholder={t("projects.path")}
    bind:value={path}
    spellcheck="false"
    autocomplete="off"
    oninput={() => (error = null)}
    onkeydown={(e) => e.key === "Escape" && path && (e.stopPropagation(), (path = ""))}
  />
  <button class="btn" type="submit" disabled={!path.trim() || busy}>{t("projects.add")}</button>
</form>
{#if error}<p class="err tone-bad">{error}</p>{/if}

<style>
  .add {
    display: flex;
    gap: var(--s2);
  }
  .field {
    font-size: 13px;
  }
  .add .btn {
    height: auto;
  }
  .err {
    margin: var(--s2) 0 0;
    font-size: 13px;
  }
</style>
