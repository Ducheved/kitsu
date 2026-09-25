// Open files. Each tab keeps its own CodeMirror state (document, selection,
// undo history), so switching tabs never loses an edit. The version a buffer
// was loaded from goes back with every save, and the backend refuses the
// save if the file changed on disk in the meantime.

import { EditorState, type Text } from "@codemirror/state";
import { errorKind, errorText } from "./api";
import { app } from "./app.svelte";
import { base } from "./editor";
import { t } from "./i18n/index.svelte";

export interface Buffer {
  path: string;
  state: EditorState;
  version: string | null;
  /** The document as last loaded or saved; compared only when the text changes. */
  saved: Text;
  dirty: boolean;
  conflict: boolean;
}

class Buffers {
  open = $state<Buffer[]>([]);
  active = $state<string | null>(null);
  /** Whose files these are. Another project's tabs wait in `parked`,
   *  unsaved edits and undo history included, until you switch back. */
  private repo = "";
  private parked = new Map<string, { open: Buffer[]; active: string | null }>();

  switchTo(repo: string) {
    if (repo === this.repo) return;
    if (this.repo) this.parked.set(this.repo, { open: this.open, active: this.active });
    const next = this.parked.get(repo);
    this.parked.delete(repo);
    this.repo = repo;
    this.open = next?.open ?? [];
    this.active = next?.active ?? null;
  }

  private get api() {
    return app.apiFor(this.repo);
  }

  get current(): Buffer | undefined {
    return this.open.find((b) => b.path === this.active);
  }

  async show(path: string) {
    if (!this.open.some((b) => b.path === path)) {
      try {
        const repo = this.repo;
        const f = await this.api.readFile(path);
        // Switched projects while it loaded: this file belongs to the other one.
        if (repo !== this.repo) return;
        const state = EditorState.create({ doc: f.text, extensions: base(path, { vim: app.prefs.vim, dark: app.dark }) });
        this.open.push({ path, state, version: f.version, saved: state.doc, dirty: false, conflict: false });
      } catch (e) {
        app.notify(errorText(e), "bad");
        return;
      }
    }
    this.active = path;
  }

  update(path: string, state: EditorState) {
    const b = this.open.find((x) => x.path === path);
    if (!b) return;
    const changed = state.doc !== b.state.doc;
    b.state = state;
    if (changed) b.dirty = !state.doc.eq(b.saved);
  }

  async save(path = this.active) {
    const b = this.open.find((x) => x.path === path);
    if (!b) return;
    const doc = b.state.doc;
    const text = doc.toString();
    try {
      b.version = await this.api.writeFile(b.path, text, b.version);
      b.saved = doc;
      // Typing may have continued while the write was in flight.
      b.dirty = !b.state.doc.eq(doc);
      b.conflict = false;
      app.notify(t("editor.saved", { path: b.path }), "ok");
    } catch (e) {
      if (errorKind(e) === "conflict") b.conflict = true;
      app.notify(errorText(e), "bad");
    }
  }

  async reload(path: string) {
    const i = this.open.findIndex((x) => x.path === path);
    if (i < 0) return;
    this.open.splice(i, 1);
    await this.show(path);
  }

  close(path = this.active) {
    const i = this.open.findIndex((x) => x.path === path);
    if (i < 0) return;
    const b = this.open[i]!;
    if (b.dirty && !confirm(t("editor.closeConfirm", { path: b.path }))) return;
    this.open.splice(i, 1);
    if (this.active === path) this.active = this.open[Math.min(i, this.open.length - 1)]?.path ?? null;
  }

  cycle(delta: number) {
    if (!this.open.length) return;
    const i = Math.max(0, this.open.findIndex((b) => b.path === this.active));
    this.active = this.open[(i + delta + this.open.length) % this.open.length]!.path;
  }
}

export const buffers = new Buffers();
app.onSwitch((repo) => buffers.switchTo(repo));
