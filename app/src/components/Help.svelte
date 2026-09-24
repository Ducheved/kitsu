<script lang="ts">
  import { app } from "../lib/app.svelte";

  const groups: [string, [string, string][]][] = [
    ["Anywhere", [["⌘K  or  :", "commands, tasks, files"], ["⌘P", "open a file"], ["n", "new task"], ["g r", "rules"], ["g h", "home"], ["?", "this"], ["Esc", "close / back"]]],
    ["Task list", [["j  k", "move"], ["⏎  o", "open"], ["/", "filter"]]],
    ["On a task", [["r", "start the agent"], ["s", "stop it"], ["1 2", "answer its question"], ["a", "accept and close"], ["c", "continue with a note"], ["x", "discard the attempt"]]],
    ["Editor", [[":w  :q  :e path", "vim ex commands"], ["⌘S", "save (vim off)"]]],
  ];
</script>

<div class="scrim" role="presentation" onclick={() => (app.overlay = null)}>
  <div class="help" role="dialog" aria-label="Keyboard shortcuts" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.overlay = null)}>
    <h2>Keys</h2>
    <div class="cols">
      {#each groups as [title, keys] (title)}
        <section>
          <h3>{title}</h3>
          {#each keys as [k, what] (k)}
            <div class="k"><span class="mono key">{k}</span><span>{what}</span></div>
          {/each}
        </section>
      {/each}
    </div>
    <label class="vim">
      <input
        type="checkbox"
        checked={app.prefs.vim}
        onchange={(e) => {
          app.prefs.vim = (e.currentTarget as HTMLInputElement).checked;
          app.savePrefs();
        }}
      />
      Vim keys in the editor
    </label>
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
  }
  .help {
    width: min(720px, 94vw);
    padding: 24px 28px;
    border-radius: 16px;
    background: var(--elev);
    box-shadow: var(--shadow);
    border: 1px solid var(--line);
  }
  h2 {
    margin: 0 0 12px;
    font-size: 18px;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 18px 32px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--faint);
  }
  .k {
    display: flex;
    gap: 14px;
    padding: 3px 0;
    font-size: 13.5px;
  }
  .key {
    width: 120px;
    color: var(--muted);
  }
  .vim {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 18px;
    color: var(--muted);
  }
</style>
