+++
title = "The window can't read, write or run arbitrary things"
scope = ["app/src-tauri/**"]
checks = ["webview-powers"]
decision = "tauri-window"
+++
The webview renders text written by agents. If that text ever runs as
script, it inherits whatever the window can do. So the window can do very
little: named commands with validated arguments, repo-relative paths
checked against the root, no fs/shell/http plugins.
