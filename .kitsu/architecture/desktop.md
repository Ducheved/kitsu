+++
title = "Desktop shell"
level = "container"
parent = "kitsu"
technology = "Tauri 2, Rust"
paths = ["app/src-tauri/**"]
uses = [{ to = "cli", why = "links the kitsu crate; starts ordinary `kitsu run` processes" }]
+++
Must never give the webview generic fs, shell, http or process powers:
typed commands only.
