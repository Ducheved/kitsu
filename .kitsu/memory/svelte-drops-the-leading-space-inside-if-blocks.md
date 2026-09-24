+++
title = "Svelte drops the leading space inside {#if} blocks"
kind = "gotcha"
scope = ["app/src/**"]
anchors = ["app/package.json"]
+++
Markup like `{#if x} · {label}{/if}` renders as "…main· 82K": the space after `}` is trimmed. Write `{" · "}` instead. Hit twice while adding token counts and memory anchors (ReviewCard, RulesPage); the string tests can't catch it, only a render can.
