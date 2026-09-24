+++
title = "Speak English, Russian, Japanese, German and French"
scope = ["app/src/**", "crates/kitsu/src/status.rs", "crates/kitsu/src/digest.rs"]
checks = ["ui", "test"]
+++
Every string a person reads in the window comes from a translation table,
picked from the system language with a manual override. Statuses and "since
you last looked" lines are rendered from structured data, not from English
sentences built in Rust. Briefs stay English: they're for agents.
