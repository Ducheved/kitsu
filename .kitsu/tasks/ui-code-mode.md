+++
title = "An editor-first layout for people who live in their editor"
scope = ["app/src/**", "app/src-tauri/src/commands.rs"]
checks = ["ui", "test"]
after = ["ui-i18n"]
+++
Two layouts over the same state. Work: the task list and one task in focus
(what exists now). Code: file tree, tabs, editor, a status bar, and a thin
strip of agent activity that only speaks up when something needs you.
Adaptive (the default) switches by what you're doing: open a file, you're in
Code; open a task, you're in Work. Every part reachable from the keyboard.
