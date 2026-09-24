+++
title = "Tauri v2 window with a web UI; editor is CodeMirror 6"
rejected = [
  "A native GPU UI (GPUI-style): the bottleneck here is not rendering text, it's knowing what the agents did; and it costs a year",
  "Monaco: ~5x the bundle of CodeMirror and a weaker vim story",
  "Embedding Neovim: a second authority for buffers, undo and selections",
]
+++
The window is a projection of state, not the owner of it. It asks typed
commands for data and gets a dirty flag when something changed. The editor
is one more projection of files on disk; saves carry the version they were
loaded from.

Kill criteria: typing p95 over 16 ms in the editor on a 10k-line file,
diff open over 150 ms for a 2k-line change, or UI refresh over 50 ms with
the 100-run stress profile. None measured over budget so far (refresh: 2 ms
in the store; webview timings not yet measured on real hardware).
