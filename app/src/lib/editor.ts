// CodeMirror setup shared by the editor and the diff view. The editor is
// one projection of a file on disk; it never becomes a second authority:
// saves carry the version they were loaded from and are refused if the
// file moved underneath (see write_file in commands.rs).

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { bracketMatching, defaultHighlightStyle, foldGutter, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, type Extension } from "@codemirror/state";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { Vim, vim } from "@replit/codemirror-vim";

export function language(path: string): Extension {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  if (["js", "mjs", "cjs", "jsx"].includes(ext)) return javascript({ jsx: true });
  if (["ts", "tsx", "mts"].includes(ext)) return javascript({ typescript: true, jsx: ext === "tsx" });
  if (ext === "py") return python();
  if (ext === "rs") return rust();
  if (ext === "json") return json();
  if (["md", "markdown"].includes(ext)) return markdown();
  return [];
}

export const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13.5px", backgroundColor: "var(--bg)", color: "var(--text)" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.6" },
  ".cm-content": { caretColor: "var(--accent)", padding: "12px 0" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--faint)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--hover)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--muted)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": { backgroundColor: "var(--accent-soft) !important" },
  ".cm-cursor, .cm-fat-cursor": { borderLeftColor: "var(--accent)" },
  "&:not(.cm-focused) .cm-fat-cursor": { outline: "1px solid var(--accent)", background: "none" },
  ".cm-fat-cursor": { background: "var(--accent) !important", color: "var(--bg) !important" },
  ".cm-panels": { backgroundColor: "var(--rail)", color: "var(--text)", borderTop: "1px solid var(--line)" },
  ".cm-vim-panel": { padding: "2px 10px", fontFamily: "var(--mono)" },
  ".cm-vim-panel input": { color: "var(--text)", fontFamily: "var(--mono)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", border: "none", color: "var(--muted)" },
});

/** Parts that change with preferences, swapped without losing the document. */
export const vimSlot = new Compartment();
export const highlightSlot = new Compartment();

export const vimExt = (on: boolean): Extension => (on ? vim({ status: true }) : []);
export const highlightExt = (dark: boolean): Extension =>
  syntaxHighlighting(dark ? oneDarkHighlightStyle : defaultHighlightStyle, { fallback: true });

export function base(path: string, opts: { vim: boolean; dark: boolean; readOnly?: boolean }): Extension[] {
  return [
    // Vim has to come before other keymaps to see keys first.
    vimSlot.of(vimExt(opts.vim)),
    lineNumbers(),
    highlightActiveLineGutter(),
    foldGutter(),
    history(),
    drawSelection(),
    indentOnInput(),
    bracketMatching(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    highlightSlot.of(highlightExt(opts.dark)),
    keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
    language(path),
    theme,
    EditorView.editable.of(!opts.readOnly),
  ];
}

// Ex commands are global to the Vim module. They act on whichever editor
// registered itself last (there is one editor on screen at a time).
export interface ExTarget {
  save: () => void;
  close: () => void;
  open: (path: string) => void;
  next: () => void;
  prev: () => void;
}
let target: ExTarget | null = null;
let defined = false;

export function setExTarget(t: ExTarget | null) {
  target = t;
  if (defined) return;
  defined = true;
  Vim.defineEx("write", "w", () => target?.save());
  Vim.defineEx("quit", "q", () => target?.close());
  Vim.defineEx("wq", "wq", () => {
    target?.save();
    target?.close();
  });
  Vim.defineEx("xit", "x", () => {
    target?.save();
    target?.close();
  });
  Vim.defineEx("edit", "e", (_cm: unknown, params: { args?: string[] }) => {
    const p = params.args?.[0];
    if (p) target?.open(p);
  });
  Vim.defineEx("bnext", "bn", () => target?.next());
  Vim.defineEx("bprevious", "bp", () => target?.prev());
  Vim.defineEx("bdelete", "bd", () => target?.close());
}
