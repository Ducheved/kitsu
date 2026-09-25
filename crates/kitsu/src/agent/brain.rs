//! The loop that calls the model.
//!
//! It owns the conversation and the budgets and nothing else: every tool,
//! every write to the journal and every decision about done goes through the
//! host over `HostLink`. It can be restarted from the journal at any point.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;

use super::context::{self, Call, Prefix, Step};
use super::protocol::{self, HostLink};
use super::provider::{OpenAiChat, ProviderError};
use crate::util::content_id;

/// Attempts per model request for errors worth retrying.
const ATTEMPTS: u32 = 5;

/// Why the loop ended. `Failed` ends the run as failed; `Stopped` as
/// finished with that reason.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    Stopped(String),
    Failed(String),
}

pub async fn run(link: &HostLink, provider: &OpenAiChat, cancel: watch::Receiver<bool>) -> End {
    match drive(link, provider, cancel).await {
        Ok(e) => e,
        Err(e) => End::Failed(e),
    }
}

async fn drive(
    link: &HostLink,
    provider: &OpenAiChat,
    mut cancel: watch::Receiver<bool>,
) -> Result<End, String> {
    let session = link.call(protocol::SESSION, json!({})).await?;
    let run = session["run"].as_str().unwrap_or("run").to_string();
    let prefix = Prefix {
        harness: session["prefix"]["harness"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        brief: session["prefix"]["brief"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    };
    let tools = session["tools"].clone();
    let tools_bytes = tools.to_string().len();
    let window = session["model"]["window"].as_u64().unwrap_or(128_000);
    let max_output = session["model"]["max_output"].as_u64().unwrap_or(16_000);
    // What a request may hold, and the verbatim tail compaction keeps.
    let budget = window.saturating_sub(max_output);
    let tail = (budget / 4).min(20_000);
    let max_turns = session["budgets"]["turns"].as_u64().unwrap_or(150);
    let max_tokens = session["budgets"]["tokens"].as_u64().unwrap_or(u64::MAX);

    let entries = link
        .call(protocol::JOURNAL_READ, json!({ "after_seq": 0 }))
        .await?;
    let entries: Vec<Value> = entries["entries"].as_array().cloned().unwrap_or_default();
    // Budgets are per run: a resumed run gets its own, counted from here.
    let this_run = |e: &&Value| e["run"].as_str().is_none_or(|r| r == run);
    let mut tokens: u64 = entries
        .iter()
        .filter(this_run)
        .filter(|e| e["kind"] == "model.response")
        .map(|e| {
            e["body"]["usage"]["prompt"].as_u64().unwrap_or(0)
                + e["body"]["usage"]["completion"].as_u64().unwrap_or(0)
        })
        .sum();
    let mut conv = context::fold(&entries);
    let earlier_turns = entries
        .iter()
        .filter(|e| !this_run(e))
        .filter(|e| e["kind"] == "model.response")
        .count() as u64;
    let mut last_prompt: Option<u64> = None;
    // Set after the provider said the last request didn't fit: compact
    // harder and retry once; a second overflow in a row ends the run.
    let mut overflowed = false;
    let mut loops = super::loops::Detector::default();
    let mut tree = String::new();

    loop {
        if *cancel.borrow() {
            return Ok(End::Stopped("cancelled".into()));
        }
        // Results for the last step's calls, in order. After a crash these
        // are the ones that never got a result; the host settles them.
        for c in conv.pending() {
            let r = call_tool(link, &c, None, false).await?;
            if let Some(t) = &r.tree {
                tree = t.clone();
            }
            let mut text = r.text;
            match loops.observe(&c.name, &c.arguments, &tree) {
                Some(1) => {
                    append(
                        link,
                        "loop.signal",
                        json!({ "call": c.id, "n": 1, "note": super::loops::WARNING }),
                    )
                    .await?;
                    text = format!("{text}\n{}", super::loops::WARNING);
                }
                Some(n) => {
                    append(link, "loop.signal", json!({ "call": c.id, "n": n })).await?;
                    conv.set_result(&c.id, text);
                    return Ok(stop(link, "doom_loop").await);
                }
                None => {}
            }
            conv.set_result(&c.id, text);
            if let Some(stop) = r.stop {
                return Ok(End::Stopped(stop));
            }
        }
        if conv.turns().saturating_sub(earlier_turns) >= max_turns {
            return Ok(stop(link, "budget_turns").await);
        }
        if tokens >= max_tokens {
            return Ok(stop(link, "budget_tokens").await);
        }

        let turn = conv.turns() + 1;
        let run_turn = turn - earlier_turns;
        let mut warnings = Vec::new();
        if run_turn * 5 >= max_turns * 4 {
            warnings.push(format!(
                "turn {run_turn} of {max_turns}; wrap up or say what blocks you"
            ));
        }
        if tokens.saturating_mul(5) >= max_tokens.saturating_mul(4) {
            warnings.push(format!("{tokens} of {max_tokens} tokens used"));
        }
        let ledger = link
            .call(protocol::STATE, json!({ "turn": run_turn, "tokens": tokens, "context": last_prompt, "warnings": warnings }))
            .await?;
        let ledger = ledger["text"].as_str().unwrap_or("").to_string();
        let mut msgs = context::render(&prefix, &conv, &ledger);
        let est = context::estimate(&msgs, tools_bytes);
        // After a real overflow only the newest step is kept verbatim.
        let trigger = if overflowed {
            Some(("overflow", budget * 2 / 5, 0))
        } else if est.saturating_mul(5) > budget.saturating_mul(4) {
            Some(("threshold", budget / 2, tail))
        } else {
            None
        };
        if let Some((why, target, tail)) = trigger {
            let c = context::compact(&prefix, &conv, &ledger, tools_bytes, target, tail);
            let shrank = (!c.elided.is_empty() || c.cut > conv.cut) && c.after < c.before;
            if shrank {
                append(
                    link,
                    "ctx.compacted",
                    json!({
                        "k": conv.compactions + 1, "trigger": why, "turn": turn,
                        "elided": c.elided.iter().map(|(call, pointer)| json!({ "call": call, "pointer": pointer })).collect::<Vec<_>>(),
                        "cut": c.cut, "digest": c.digest, "before": c.before, "after": c.after, "algo": "v1",
                    }),
                )
                .await?;
                conv.apply(&c);
                msgs = context::render(&prefix, &conv, &ledger);
            } else if overflowed {
                // It didn't fit and there's nothing left to shrink: sending
                // the same request again would only overflow again.
                return Ok(stop(link, "context_overflow").await);
            }
        }
        let body = provider.body(&msgs, &tools);
        let request_sha = content_id(body.to_string().as_bytes());

        let reply = match request(link, provider, &body, turn, &mut cancel).await? {
            Ok(r) => r,
            Err(ProviderError::Cancelled) => return Ok(End::Stopped("cancelled".into())),
            Err(ProviderError::Overflow(_)) if overflowed => {
                return Ok(stop(link, "context_overflow").await);
            }
            Err(ProviderError::Overflow(_)) => {
                overflowed = true;
                continue;
            }
            Err(e) => return Ok(End::Failed(format!("model provider: {}", e.detail()))),
        };

        let calls: Vec<Call> = reply
            .calls
            .iter()
            .enumerate()
            .map(|(i, c)| Call {
                id: format!("{run}/c{}", conv.calls_issued + 1 + i as u64),
                provider_id: c.id.clone(),
                name: c.name.clone(),
                arguments: c.arguments.clone(),
                result: None,
                elided: None,
            })
            .collect();
        let used = reply.usage.prompt.unwrap_or(0) + reply.usage.completion.unwrap_or(0);
        tokens += used;
        last_prompt = reply.usage.prompt;
        overflowed = false;
        append(
            link,
            "model.response",
            json!({
                "turn": turn,
                "text": reply.text,
                "calls": calls.iter().map(|c| json!({ "call": c.id, "provider_id": c.provider_id, "name": c.name, "arguments": c.arguments })).collect::<Vec<_>>(),
                "finish": reply.finish,
                "usage": { "prompt": reply.usage.prompt, "completion": reply.usage.completion, "cached": reply.usage.cached, "cost": reply.usage.cost },
                "request_sha": request_sha,
            }),
        )
        .await?;
        conv.calls_issued += calls.len() as u64;
        let no_calls = calls.is_empty();
        conv.steps.push(Step {
            turn,
            text: reply.text.clone(),
            calls,
            notice: None,
        });

        // A reply with nothing to do is the model saying it's done. That is a
        // request for verification, same as calling finish.
        if no_calls {
            let args =
                json!({ "outcome": "done", "summary": reply.text.clone().unwrap_or_default() })
                    .to_string();
            let implicit = Call {
                id: format!("{run}/c{}", conv.calls_issued + 1),
                provider_id: String::new(),
                name: "finish".into(),
                arguments: args,
                result: None,
                elided: None,
            };
            conv.calls_issued += 1;
            let r = call_tool(link, &implicit, Some(turn), true).await?;
            if let Some(s) = conv.steps.last_mut() {
                s.notice = Some(r.text);
            }
            if let Some(stop) = r.stop {
                return Ok(End::Stopped(stop));
            }
        }
    }
}

struct ToolResult {
    text: String,
    stop: Option<String>,
    tree: Option<String>,
}

async fn call_tool(
    link: &HostLink,
    c: &Call,
    turn: Option<u64>,
    implicit: bool,
) -> Result<ToolResult, String> {
    let r = link
        .call(
            protocol::TOOLS_CALL,
            json!({ "name": c.name, "arguments": c.arguments, "_meta": { "kitsu/call": c.id, "kitsu/turn": turn, "kitsu/implicit": implicit } }),
        )
        .await?;
    let text = r["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ToolResult {
        text,
        stop: r["_meta"]["kitsu/stop"].as_str().map(str::to_string),
        tree: r["_meta"]["kitsu/tree"].as_str().map(str::to_string),
    })
}

async fn append(link: &HostLink, kind: &str, body: Value) -> Result<(), String> {
    link.call(
        protocol::JOURNAL_APPEND,
        json!({ "kind": kind, "body": body }),
    )
    .await
    .map(|_| ())
}

async fn stop(link: &HostLink, reason: &str) -> End {
    let _ = link.call(protocol::STOP, json!({ "reason": reason })).await;
    End::Stopped(reason.into())
}

/// One model request, retried only for errors that say retrying can help,
/// with a bounded number of attempts. Each failed attempt is journaled.
async fn request(
    link: &HostLink,
    provider: &OpenAiChat,
    body: &Value,
    turn: u64,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Result<super::provider::Reply, ProviderError>, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let e = match provider.complete(body, cancel).await {
            Ok(r) => return Ok(Ok(r)),
            Err(e) => e,
        };
        let wait = match &e {
            ProviderError::RateLimited { retry_after, .. } => {
                Some(retry_after.unwrap_or(backoff(attempt)))
            }
            ProviderError::Transient(_) => Some(backoff(attempt)),
            _ => None,
        };
        append(
            link,
            "model.error",
            json!({ "turn": turn, "class": e.class(), "attempt": attempt, "detail": e.detail(), "retry_in_ms": wait.map(|w| w.as_millis() as u64) }),
        )
        .await?;
        match wait {
            Some(w) if attempt < ATTEMPTS => {
                tokio::select! {
                    _ = tokio::time::sleep(w) => {}
                    _ = cancel.wait_for(|c| *c) => return Ok(Err(ProviderError::Cancelled)),
                }
            }
            _ => return Ok(Err(e)),
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let base = std::env::var("KITSU_BACKOFF_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000);
    Duration::from_millis(base.saturating_mul(1 << (attempt - 1).min(4)))
}
