+++
title = "Make stop, orphan cleanup and checks work on Windows"
scope = ["crates/kitsu/src/runner.rs", "crates/kitsu/src/recover.rs", "crates/kitsu/src/workspace.rs", "crates/kitsu/src/check.rs"]
checks = ["test"]
+++
The wake-up is SIGUSR1 and orphan reaping reads /proc; on Windows the 1 Hz
poll still works but stop takes up to a second, and orphans are reported,
not stopped. Use a named event for wake-ups and a job object per run.
CI needs a Windows runner before any of this counts.
