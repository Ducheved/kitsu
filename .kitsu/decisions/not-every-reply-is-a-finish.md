+++
title = "A reply without a tool call is a finish only the second time in a row; a cut-off or empty reply never is"
scope = ["crates/kitsu/src/agent/brain.rs", "crates/kitsu/src/agent/context.rs", "fixtures/stub-model/**"]
state = "accepted"
rejected = [
  "Taking every reply without a call as finish (what the loop did): a first turn that narrates (\"Let me look at the code.\") stopped the run unverified before it started, and a reply cut off at max_output was verified as if the model were through",
  "Failing every call of a cut-off reply, as Pi does: a call whose arguments parse was complete when the limit hit; only the ones that don't parse are refused, and the parser already refuses them",
  "Retrying a cut-off reply: the same request with the same limit is cut in the same place; the model has to send less, so it's told",
  "An empty reply in the conversation: it has nothing to answer, and some providers refuse an assistant message with neither content nor calls",
]
+++
What the loop does with a reply, in order:

- **Neither text nor a call** (after dropping call slots with no name and
  no arguments): retried like a transient provider error, journaled as a
  `model.error`, at most five attempts; then the run fails with "empty
  reply". It never enters the conversation.
- **Stopped at the output limit** (`length`, `max_tokens`,
  `max_output_tokens`): calls whose arguments parse run; a call whose
  arguments don't gets "invalid arguments" plus a note that the output
  limit cut it, and nothing runs. A reply with no calls gets the same note
  as a message and is not a finish. Three cut-off replies in a row stop the
  run with `output_limit`.
- **Text, no call**: the first gets a message ("call a tool, or finish"),
  journaled as `loop.signal` kind `no_call`; the next one right after it is
  the model saying it's done, and goes to `finish` as before.

Every note is a `loop.signal` (`kind`, `note`, and `call` or `turn`), so a
resumed run rebuilds the same conversation. `finish` itself, its checks and
its three refusals are unchanged: done is still decided by the checks.
