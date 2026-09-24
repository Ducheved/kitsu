<script lang="ts">
  // The Code layout's view of the agents: only what's working or needs you,
  // with the one-key answers right here so you don't have to leave the file.
  import { app } from "../lib/app.svelte";
  import { api, errorText } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { inline } from "../lib/md";
  import { statusText } from "../lib/status";
  import Glyph from "./Glyph.svelte";

  const tasks = $derived((app.overview?.tasks ?? []).filter((x) => x.attention === "needs_you" || x.attention === "working"));
  const asks = $derived(app.overview?.asks ?? []);

  async function reply(id: number, option: string) {
    try {
      await api.answerAsk(id, option);
      app.changed();
    } catch (e) {
      app.notify(errorText(e), "bad");
    }
  }
</script>

<aside class="strip">
  <header><span class="title">{t("strip.title")}</span></header>
  <div class="list scroll">
    {#each tasks as task (task.id)}
      {@const ask = "run" in task.status ? asks.find((a) => a.run === (task.status as { run: string }).run) : undefined}
      <div class="item" class:asking={!!ask}>
        <button class="head" onclick={() => app.openTask(task.id)}>
          <Glyph attention={task.attention} status={task.status} />
          <span class="text">
            <span class="t">{task.title}</span>
            <span class="r">{statusText(task)}</span>
          </span>
        </button>
        {#if ask}
          <div class="ask">
            <div class="q">{@html inline(ask.request.title)}</div>
            <div class="opts">
              {#each ask.request.options as o (o.optionId)}
                <button class="btn mini {o.kind?.startsWith('allow') ? 'primary' : ''}" onclick={() => reply(ask.id, o.optionId)}>{o.name ?? o.optionId}</button>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {:else}
      <div class="quiet hint">{t("strip.quiet")}</div>
    {/each}
  </div>
</aside>

<style>
  .strip {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--rail);
    border-left: 1px solid var(--line);
  }
  header {
    padding: 14px 14px 8px;
  }
  .title {
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--faint);
  }
  .list {
    flex: 1;
    min-height: 0;
    padding: 0 8px 12px;
  }
  .item {
    margin-bottom: 4px;
    border-radius: 10px;
  }
  .item.asking {
    background: var(--elev);
    box-shadow: 0 0 0 1px var(--warn);
  }
  .head {
    display: flex;
    gap: 9px;
    align-items: flex-start;
    width: 100%;
    padding: 8px 10px;
    border-radius: 10px;
    text-align: left;
  }
  .head:hover {
    background: var(--hover);
  }
  .head :global(.glyph) {
    margin-top: 3px;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 500;
  }
  .r {
    color: var(--muted);
    font-size: 12px;
  }
  .ask {
    padding: 0 10px 10px 33px;
    font-size: 12.5px;
  }
  .q {
    margin-bottom: 6px;
  }
  .opts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .mini {
    height: 26px;
    padding: 0 10px;
    font-size: 12.5px;
  }
  .quiet {
    padding: 10px;
  }
</style>
