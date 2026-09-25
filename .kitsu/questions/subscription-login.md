+++
title = "Sign in with a Claude or ChatGPT subscription in Kitsu's own loop?"
blocks = []
+++
Asked for: `kitsu login` with a subscription ("вход по подписке / OAuth")
instead of a pasted API key, for Anthropic and OpenAI as well as
OpenRouter. What the vendors' own documents say (read 2026-09-25):

**Anthropic: not offered to third parties, and forbidden.**
- Claude Code's legal page: "OAuth authentication is intended exclusively
  for purchasers of Claude Free, Pro, Max, Team, and Enterprise
  subscription plans and is designed to support ordinary use of Claude
  Code and other native Anthropic applications ... Anthropic does not
  permit third-party developers to offer Claude.ai login into their own
  applications, or to route requests through Free, Pro, or Max plan
  credentials on behalf of their users. Moreover, developers may not
  collect, store, or intermediate Claude.ai credentials or session tokens."
  https://code.claude.com/docs/en/legal-and-compliance.md
- Agent SDK overview: "Unless previously approved, Anthropic does not allow
  third party developers to offer claude.ai login or rate limits for their
  products." https://code.claude.com/docs/en/agent-sdk/overview.md
- The Claude API authenticates with an API key (`Authorization: Bearer`),
  Workload Identity Federation (for workloads, not people) or App Attest
  (registered iOS/macOS apps).
  https://platform.claude.com/docs/en/manage-claude/authentication.md
  Console sign-in (`ant auth login`) exists for Anthropic's own CLI, with
  no documented client registration for other apps.
  https://platform.claude.com/docs/en/cli-sdks-libraries/cli/authentication.md

**OpenAI: "Sign in with ChatGPT" is documented only for OpenAI's own
clients.** https://learn.chatgpt.com/docs/auth.md lists it for the ChatGPT
desktop app, Codex CLI and the IDE extension; the API is used with an API
key (or Workload Identity Federation for workloads,
https://developers.openai.com/api/docs/guides/workload-identity-federation.md).
Nothing in OpenAI's developer docs offers third-party apps a sign-in that
carries a ChatGPT plan's usage. (Third-party tools that do it reuse Codex
CLI's client id; that is exactly what not to do.)

**OpenRouter: offered.** OAuth PKCE for third-party apps, localhost
callbacks on any port, exchanging the code for a user-controlled key
(https://openrouter.ai/docs/guides/overview/auth/oauth.md). That one is
implemented: `kitsu login openrouter`, `auth = "login:openrouter"`
(decision `native-providers`). The key works for all three of Kitsu's wire
formats through OpenRouter.

**The legitimate way to use a subscription today:** run the vendor's own
agent. Kitsu already drives Claude Code and Codex over ACP
(`kitsu run <task> --agent claude` / `--agent codex`); they sign in with
the subscription themselves (`claude` → /login, `codex login`), and Kitsu
never sees the credential.

Nothing is blocked. The question for you: is that enough, or should Kitsu
ask Anthropic (sales, "previously approved") or OpenAI for a third-party
sign-in? Until one of them publishes such a flow, Kitsu's own loop takes an
API key or an OpenRouter login.
