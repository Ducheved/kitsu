<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { type Key, t } from "../lib/i18n/index.svelte";
  import { tour } from "../lib/tour.svelte";
  import Fox from "./Fox.svelte";

  const groups: [Key, [string, Key][]][] = [
    [
      "help.anywhere",
      [
        ["⌘K  :", "help.commands"],
        ["⌘P", "help.openFile"],
        ["n", "help.newTask"],
        ["g r", "help.rules"],
        ["g h", "help.home"],
        ["g p", "help.plan"],
        ["⌘1  ⌘2", "help.layouts"],
        ["⌘,", "help.settings"],
        ["?", "help.this"],
        ["Esc", "help.back"],
      ],
    ],
    [
      "help.projects",
      [
        ["⌘O", "help.switcher"],
        ["⌥1 … ⌥9", "help.projectN"],
        ["g b", "help.tree"],
      ],
    ],
    ["help.list", [["j  k", "help.move"], ["⏎  o", "help.open"], ["/", "help.filter"]]],
    [
      "help.task",
      [
        ["r", "help.start"],
        ["s", "help.stop"],
        ["1 2", "help.answer"],
        ["a", "help.accept"],
        ["c", "help.continue"],
        ["x", "help.discard"],
      ],
    ],
    [
      "help.code",
      [
        ["⌘B", "help.toggleTree"],
        ["⌘J", "help.toggleStrip"],
        ["⌃⇥  :bn  :bp", "help.tabs"],
        [":bd", "help.closeTab"],
        ["j k h l ⏎", "help.treeKeys"],
      ],
    ],
    ["help.editor", [[":w  :q  :e path", "help.ex"], ["⌘S", "help.save"]]],
    [
      "help.planGroup",
      [
        ["← → ↑ ↓", "help.planMove"],
        ["⏎", "help.open"],
        ["n", "help.newTask"],
        ["e", "help.planEdit"],
        ["⇥  ⌫", "help.planLinks"],
        ["⌘Z", "help.planUndo"],
        ["f  +  −", "help.planFit"],
        ["d", "help.planDim"],
      ],
    ],
  ];
</script>

<div class="scrim" role="presentation" onclick={() => (app.overlay = null)}>
  <div class="help" role="dialog" aria-label={t("cmd.keys")} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.overlay = null)}>
    <h2>{t("help.title")}</h2>
    <div class="cols">
      {#each groups as [title, keys] (title)}
        <section>
          <h3>{t(title)}</h3>
          {#each keys as [k, what] (k)}
            <div class="k"><span class="mono key">{k}</span><span>{t(what)}</span></div>
          {/each}
        </section>
      {/each}
    </div>
    <div class="foot">
      <label class="vim">
        <input
          type="checkbox"
          checked={app.prefs.vim}
          onchange={(e) => {
            app.prefs.vim = (e.currentTarget as HTMLInputElement).checked;
            app.savePrefs();
          }}
        />
        {t("settings.vim")}
      </label>
      <button
        class="btn learn"
        onclick={() => {
          app.overlay = null;
          tour.start();
        }}><Fox size={22} />{t("cmd.tour")}</button
      >
    </div>
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
    animation: fade-in var(--fast) var(--ease);
  }
  .help {
    width: min(720px, 94vw);
    max-height: 92vh;
    overflow: auto;
    padding: var(--s5) var(--s6);
    border-radius: 16px;
    background: var(--elev);
    box-shadow: var(--shadow);
    border: 1px solid var(--line);
    animation: pop-in var(--quick) var(--ease);
  }
  h2 {
    margin: 0 0 var(--s4);
    font-size: 18px;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--s5) var(--s6);
  }
  h3 {
    margin: 0 0 var(--s2);
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
    flex: none;
    width: 120px;
    color: var(--muted);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
    margin-top: var(--s5);
    padding-top: var(--s4);
    border-top: 1px solid var(--line);
  }
  .vim {
    display: flex;
    gap: var(--s2);
    align-items: center;
    color: var(--muted);
  }
  .learn {
    padding-left: var(--s2);
  }
</style>
