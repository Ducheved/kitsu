+++
title = "Kitsu's own loop speaks Chat Completions, Anthropic Messages and OpenAI Responses; a key can come from `kitsu login`, kept only in the OS keychain"
scope = ["crates/kitsu/src/agent/**", "crates/kitsu/src/agents.rs", "crates/kitsu/src/cli.rs", "fixtures/stub-model/**"]
state = "accepted"
rejected = [
  "A provider trait with a crate per vendor (async-openai, an Anthropic SDK): three enum arms and one SSE loop are the whole difference, and an SDK would own the retry and error mapping the brain already owns",
  "A conversation per provider: resume, compaction, loop detection and the journal would each need three versions; the fold stays neutral and a provider only renders it",
  "`previous_response_id` (store: true) for Responses: the state would live at the provider, a resumed run would depend on a response that may be gone, and OpenRouter's Responses API refuses it",
  "Rebuilding reasoning items from the conversation: they're opaque (encrypted) and must go back as they came, so they're journaled with the reply",
  "Anthropic's `x-api-key` header: Anthropic documents it as legacy next to `Authorization: Bearer`, and OpenRouter's Messages endpoint documents only the bearer header",
  "Signing in with a ChatGPT or Claude subscription by reusing Codex's or Claude Code's OAuth client: neither vendor offers that to third-party apps (question `subscription-login`); Kitsu drives their own CLIs over ACP instead, and they sign in themselves",
  "A plaintext or 0600 file when there's no keychain: a key on disk is what `api_key_env` avoids; without a keychain `kitsu login` fails and says to use api_key_env",
  "The keyring crate's store picker without the `v1` defaults (keyring-core plus hand-picked stores): more code for the same three stores",
]
+++
`native = { provider = ... }` picks the wire format:

- `openai-chat` (as before): `POST {base_url}/chat/completions`.
- `anthropic-messages`: `POST {base_url}/v1/messages`, `anthropic-version:
  2023-06-01`. The harness and the brief are `system` blocks; tool calls are
  `tool_use` blocks and their results `tool_result` blocks in the next user
  turn. Every request carries three `cache_control` marks, placed as
  `native-prompt-cache` places them: the harness, the brief, and the block
  right before the ledger; the ledger is the last block and is never marked.
  Usage counts every input token (`input_tokens` is only what came after
  the last mark, so read and written cache tokens are added back), and
  records the cache read and the cache write. Errors: 529 and
  `overloaded_error` in the stream are transient, 429 is rate-limited (with
  `retry-after`), 5xx transient, 401 fatal, "prompt is too long" an
  overflow; a reply that stops with `model_context_window_exceeded` is
  handled as an overflow too, since the next request can't fit either.
- `openai-responses`: `POST {base_url}/responses`, `store: false`, tools
  with `strict: false` so the schemas mean the same for every provider.
  Output items come back verbatim in the next request's input, reasoning
  items included (their `encrypted_content` is returned by default in
  stateless mode; `include: ["reasoning.encrypted_content"]` keeps servers
  on the older contract returning it). They're journaled with the reply as
  `replay`, without the call arguments the journal already has, so a
  resumed run sends the same input. `response.failed` and `error` events
  are classified by their code.

Base URLs follow each vendor's convention (ANTHROPIC_BASE_URL has no
version, OPENAI_BASE_URL has one), so `https://openrouter.ai/api` serves
`anthropic-messages` and `https://openrouter.ai/api/v1` the other two. All
three send the key as `Authorization: Bearer`.

The key source is exactly one of `api_key_env = "VAR"` (as before) and
`auth = "login:<provider>"`, the key `kitsu login <provider>` put in the OS
keychain. The only login is OpenRouter's documented OAuth PKCE flow for
third-party apps
(https://openrouter.ai/docs/guides/overview/auth/oauth.md): S256
challenge, a loopback callback on 127.0.0.1 with a random port (OpenRouter
accepts localhost on any port), the state in the callback's path (the flow
has no `state` parameter; only that exact path is taken as the answer), a
5-minute timeout, and `POST /api/v1/auth/keys` exchanging the code and the
verifier for a key the user controls and can revoke on openrouter.ai. The
keychain is checked before the browser opens, so no key is minted that
can't be kept. The key is never printed, logged, journaled or put in the
tools' environment.

The keychain is the `keyring` crate (4.x, default `v1` stores: macOS
Keychain, Windows Credential Manager, the Secret Service over zbus on
Linux; maintained by the open-source-cooperative, 4.2.0 released
2026-08-29, MIT/Apache-2.0). On Linux it adds the zbus stack (about 55
crates, pure Rust, no libdbus). `getrandom` (already in the tree) gives the
verifier and the state.

Not verified live: no request went to api.anthropic.com, api.openai.com or
openrouter.ai's OAuth endpoints (no key or account in the session that
wrote this). The request and stream shapes come from the vendors' current
references and are exercised against the scripted stub
(`fixtures/stub-model/server.py`) in `crates/kitsu/tests/native.rs`.

Revisit if: Anthropic or OpenAI publish a sign-in for third-party apps; a
live run shows a shape the stub doesn't; the keychain dependency weighs
more than it gives (then keyring-core with one store per platform).
