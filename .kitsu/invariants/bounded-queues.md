+++
title = "Every queue has a bound"
scope = ["crates/**", "app/src-tauri/src/**"]
checks = ["bounded"]
+++
Agent output, events, logs, UI updates: all bounded. Backpressure means the
producer waits (the agent blocks on its stdout pipe), not that memory grows.
Logs keep head and tail. The UI gets a dirty flag, not a stream.
