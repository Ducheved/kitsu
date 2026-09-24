+++
title = "Webview UI"
level = "container"
parent = "kitsu"
technology = "Svelte 5, CodeMirror 6"
paths = ["app/src/**"]
uses = [{ to = "desktop", why = "typed Tauri commands" }]
+++
Owns no engineering state: it pulls a snapshot when the shell says
something changed. Editor buffers are its only state.
