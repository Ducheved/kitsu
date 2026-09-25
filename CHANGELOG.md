# Changelog

## [0.2.0](https://github.com/Ducheved/kitsu/compare/v0.1.0...v0.2.0) (2026-09-25)


### ⚠ BREAKING CHANGES

* .kitsu/invariants/ is no longer read. Put `guards` and `why` on the check in kitsu.toml and move the prose to a decision.

### Features

* **agent:** --policy triage lets the judge spare you low-risk commands ([9b831b7](https://github.com/Ducheved/kitsu/commit/9b831b7fbc40d8fdd9983b58592d6fe9d0e8dff4))
* **agent:** Anthropic Messages and OpenAI Responses providers, kitsu login openrouter ([b2f99fd](https://github.com/Ducheved/kitsu/commit/b2f99fdcd7d3fd50618e308b237d621a760e8927))
* **agent:** compact the conversation, never the brief ([8a94f2d](https://github.com/Ducheved/kitsu/commit/8a94f2dcd1ed05f8af170be0cf005edfe90b1a86))
* **agent:** Kitsu's own agent loop, with done decided by checks ([a5e1cb2](https://github.com/Ducheved/kitsu/commit/a5e1cb20bb6e7178a6e0c1778c6f4bf55b4c2ac0))
* **agent:** mark prompt-cache breakpoints for Anthropic models ([ca24a6c](https://github.com/Ducheved/kitsu/commit/ca24a6cfa3472d0fb298bbc0e2153be0da132cbe))
* **agent:** resume a run after a crash without repeating its effects ([d7cd16a](https://github.com/Ducheved/kitsu/commit/d7cd16a40b0bfe36ba650d46ee03e29740cd92d7))
* **agent:** stop going in circles, stop promptly, retry only what helps ([ec4b0bd](https://github.com/Ducheved/kitsu/commit/ec4b0bd2224e178bcdc3d867fcb2b5c9c68a81fd))
* **app:** every command names its repository ([5f46526](https://github.com/Ducheved/kitsu/commit/5f46526d9ae5a595c59c097e0a13a36284fb496a))
* **app:** receipts next to green checks, unknown paths, judgments labeled as judgments ([01c89a3](https://github.com/Ducheved/kitsu/commit/01c89a3df4540b208bda579f03a933aacf76d22d))
* **arch:** architecture model checked against the code ([a3217c3](https://github.com/Ducheved/kitsu/commit/a3217c3467e37f479f807d010f9e146be89622ef))
* **brief:** keep the rules in the agent's system prompt so compaction can't drop them ([090600e](https://github.com/Ducheved/kitsu/commit/090600e47f18d2f5ac1edf1e225bf0cb50517dfe))
* enforce kitsu.toml inside Claude Code, Codex and Cursor hooks and in CI ([e4fc06b](https://github.com/Ducheved/kitsu/commit/e4fc06bfd75c06981a7abb6ba63a646ec65604d0))
* **index:** local code search with incremental updates by blob id ([fbcf0ac](https://github.com/Ducheved/kitsu/commit/fbcf0ac39c59854ed2241855e20fe352181d916e))
* **index:** retrieval eval against a grep baseline ([d62c246](https://github.com/Ducheved/kitsu/commit/d62c246c37f3289e5f9d267119f329f9785d5380))
* **intent:** pin the line between the task graph, execution and memory ([0903576](https://github.com/Ducheved/kitsu/commit/090357685fb2a280a6a39db5233e791fc48a285b))
* **judge:** typed judgments backed by TypeSafe System One ([9fc8250](https://github.com/Ducheved/kitsu/commit/9fc82506dbedbcfb30db517e4f9d4eaf82e49a8b))
* **mcp:** give agents Kitsu's answers as tools instead of shell ([c42f3f4](https://github.com/Ducheved/kitsu/commit/c42f3f4c77337d15838b3e78c754236d7d689b0e))
* **memory:** retire and supersede notes instead of deleting them ([31ab5d5](https://github.com/Ducheved/kitsu/commit/31ab5d5c43669e4e2259eee4628a18533f5764db))
* **memory:** typed notes that go stale when their code changes ([03f3439](https://github.com/Ducheved/kitsu/commit/03f343910f40b7c0caa67809e40165a49616b47c))
* **review:** show the receipt behind every passing check and call unchecked paths unknown ([57b9e6d](https://github.com/Ducheved/kitsu/commit/57b9e6d5a10e236470cb0fa730c1f99f26e2fb7d))
* **runner:** start agents niced and off one CPU ([2a4ec25](https://github.com/Ducheved/kitsu/commit/2a4ec2508d2482f5369665c1a04544c970556f1b))
* **tasks:** edit a task's after, checks and scope in place ([cac1a34](https://github.com/Ducheved/kitsu/commit/cac1a3463e083e534fe65fa3a89caec1f2ec7dd4))
* **ui:** bigger, easier-to-see paw prints ([ee7b3ff](https://github.com/Ducheved/kitsu/commit/ee7b3ff6da73ed66ff7e58d1809b0027e363d058))
* **ui:** bundle Inter and JetBrains Mono so the app looks the same everywhere ([07e5d5a](https://github.com/Ducheved/kitsu/commit/07e5d5a506f8e026ac7a67fd9c8ec8b1a65df7e8))
* **ui:** fox paw prints in the corner of the main pane ([113605d](https://github.com/Ducheved/kitsu/commit/113605da6adac17c69e150c05c9e0a4e91c0725e))
* **ui:** many projects in one window ([f2d3364](https://github.com/Ducheved/kitsu/commit/f2d336430b9c7efd229446676f9434e6ae705758))
* **ui:** more air, micro-interactions and a Learn Kitsu tour ([56bd8a7](https://github.com/Ducheved/kitsu/commit/56bd8a7ff60f5ac33b450821d220fbb8cc7af19f))
* **ui:** Plan view, the task graph you edit by dragging ([fc33b3d](https://github.com/Ducheved/kitsu/commit/fc33b3d3a5309dd43250ead863bf7bef998c1c4d))
* **ui:** spacing scale, motion tokens and a fox mascot ([fec6ada](https://github.com/Ducheved/kitsu/commit/fec6ada2a8d45683a3a2dd5b382b26b28ee2a3cd))
* **ui:** the fox reacts to what's happening ([edc9e84](https://github.com/Ducheved/kitsu/commit/edc9e84a6cb473dac8a9ba5b87dc9cff1f32776d))
* **workspaces:** a list of projects, per-project summaries and a branches/worktrees view ([33f3860](https://github.com/Ducheved/kitsu/commit/33f38602bd7a827aebd54fbb9f26bb862c194fb3))


### Bug Fixes

* **accept:** refuse a change that breaks the rule files it would land ([c0ed2b8](https://github.com/Ducheved/kitsu/commit/c0ed2b873e8b6c2d9f99ba17fb4e2eead6fcdd35))
* **agent:** don't tell the model a leftover process was stopped on Windows ([d28abdc](https://github.com/Ducheved/kitsu/commit/d28abdc125e132e1d94f73697e106823e98e3f59))
* **agent:** harden the native loop's shell, reply handling, resume and auto policy ([2c83fdc](https://github.com/Ducheved/kitsu/commit/2c83fdcb7be758037c626f72b863ed05b1a75f5e))
* **agent:** never show a held-out check's output to the model ([6ef1cb3](https://github.com/Ducheved/kitsu/commit/6ef1cb3ffe2422c85b40705a721e52f2651867bb))
* **agents:** match environment names the way Windows does ([738be2f](https://github.com/Ducheved/kitsu/commit/738be2fa35fa7458f204e04281497aed54f9edcc))
* **app:** add the .ico and .icns icons the Windows and macOS bundles need ([8591918](https://github.com/Ducheved/kitsu/commit/859191843630e6ec9f44c2b6807ab17e7fee4d88))
* **arch:** an unreadable kitsu.toml fails `arch check` ([7e9a7b1](https://github.com/Ducheved/kitsu/commit/7e9a7b17f1182aaf730f30d80b6d1e399c551d16))
* **brief:** don't report accept-time checks as the attempt's own result ([61e6334](https://github.com/Ducheved/kitsu/commit/61e63345e5b6e4b9cc9e36d3ab9310afe6865e6c))
* build with -D warnings on targets other than Linux ([b6950ec](https://github.com/Ducheved/kitsu/commit/b6950ec1497f9014197151ae461d1b6e613c28d9))
* **check:** run commands under sh on Windows too, never cmd ([d552be1](https://github.com/Ducheved/kitsu/commit/d552be1bb3a94b8a86de3e82a3760ccd9365becd))
* **eval:** read and write text as UTF-8 with LF, and run on Windows ([f77950b](https://github.com/Ducheved/kitsu/commit/f77950b7a93af3fbf70dfa4d6f3d9bf2597e0f2a))
* **git:** evidence tree hash ignored files git was told to ignore ([48f0b6e](https://github.com/Ducheved/kitsu/commit/48f0b6e048f1116085ae1f966df9af0a30564a1a))
* **git:** keep the index mtime on the scratch copy so racy entries get re-hashed ([34ac50b](https://github.com/Ducheved/kitsu/commit/34ac50bfe7d7438366b313efc3054d73e4aa0600))
* **git:** paths git prints use the platform's separators ([0c1df56](https://github.com/Ducheved/kitsu/commit/0c1df56c065edf6d39fc3dab23f03aacf627fa06))
* **git:** stop cat-file --batch from deadlocking on large reads ([615f7f9](https://github.com/Ducheved/kitsu/commit/615f7f99645411d84bf65aaa19993e9ecbef8543))
* **git:** use a bounded channel in the cat-file deadlock test ([08e0d88](https://github.com/Ducheved/kitsu/commit/08e0d889e03a590b7f4745fedbefd6e9c98309cb))
* **runner:** judge "inside the worktree" by Windows path rules there ([32b5408](https://github.com/Ducheved/kitsu/commit/32b54080ce7f14e7d856631ec8b7c4a7853ae6ae))
* **runner:** never auto-grant a standing permission ([dccc299](https://github.com/Ducheved/kitsu/commit/dccc2997c4ced18877c930780e63d7ad0045e381))
* **runner:** start agents from an allowlisted environment ([5f43210](https://github.com/Ducheved/kitsu/commit/5f4321031f3977c39d2ffc7994e075f384dd75e7))
* **ui:** show every required check on the task page ([78478aa](https://github.com/Ducheved/kitsu/commit/78478aa6a48b1fa4a5e30422081d96df06c3b740))
* **ui:** the Plan hint says a click selects a link, ⌫ removes it ([a0f2e8a](https://github.com/Ducheved/kitsu/commit/a0f2e8a657bb5563cee184da6cda821df8dba519))


### Code Refactoring

* fold invariants into checks that say what they guard ([877aa1d](https://github.com/Ducheved/kitsu/commit/877aa1d217f47f5d3362711c77eec2ac32aab55e))
