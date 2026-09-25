//! What the model sees, as a pure function of the journal.
//!
//! ```text
//! system   the harness prompt        same bytes for every run
//! system   the brief                 per run; never compacted
//! user     "Begin…"
//! user     digest                    only after compaction
//! …        assistant / tool steps    the only part compaction touches
//! user     the ledger                re-rendered every request, never stored
//! ```
//!
//! The ledger goes last so everything before it stays byte-identical
//! between requests and the provider's prompt cache keeps hitting.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::provider::{Msg, ToolCall};

pub const BEGIN: &str = "Begin. The last message in this conversation is always Kitsu's current state of your run: what you changed, where each required check stands, how much budget is left.";

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// Kitsu's id, unique across runs: `<run>/c<n>`.
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    /// Set when compaction replaced the result with a pointer.
    pub elided: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub turn: u64,
    pub text: Option<String>,
    pub calls: Vec<Call>,
    /// What Kitsu answered to a reply with no tool calls (which it takes as
    /// `finish`). Rendered as a user message after the step.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    pub steps: Vec<Step>,
    /// Steps before this index were replaced by `digest`.
    pub cut: usize,
    pub digest: Option<String>,
    pub compactions: u64,
    /// Calls issued so far, for numbering the next one.
    pub calls_issued: u64,
}

impl Conversation {
    pub fn turns(&self) -> u64 {
        self.steps.last().map_or(0, |s| s.turn)
    }

    /// Calls of the last step that have no result yet: after a crash, or
    /// between a model response and its tools.
    pub fn pending(&self) -> Vec<Call> {
        self.steps
            .last()
            .map(|s| {
                s.calls
                    .iter()
                    .filter(|c| c.result.is_none())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_result(&mut self, call: &str, text: String) {
        for s in self.steps.iter_mut().rev() {
            if let Some(c) = s.calls.iter_mut().find(|c| c.id == call) {
                c.result = Some(text);
                return;
            }
        }
    }
}

/// Journal entries (`{kind, body}`, in order) to the conversation.
pub fn fold(entries: &[Value]) -> Conversation {
    let mut conv = Conversation::default();
    let mut results: BTreeMap<String, String> = BTreeMap::new();
    let mut elided: BTreeMap<String, String> = BTreeMap::new();
    for e in entries {
        let body = &e["body"];
        match e["kind"].as_str().unwrap_or("") {
            "model.response" => {
                let calls: Vec<Call> = body["calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| Call {
                        id: c["call"].as_str().unwrap_or("").to_string(),
                        provider_id: c["provider_id"].as_str().unwrap_or("").to_string(),
                        name: c["name"].as_str().unwrap_or("").to_string(),
                        arguments: c["arguments"].as_str().unwrap_or("").to_string(),
                        result: None,
                        elided: None,
                    })
                    .collect();
                conv.calls_issued += calls.len() as u64;
                conv.steps.push(Step {
                    turn: body["turn"].as_u64().unwrap_or(conv.steps.len() as u64 + 1),
                    text: body["text"].as_str().map(str::to_string),
                    calls,
                    notice: None,
                });
            }
            "tool.end" => {
                if let (Some(call), Some(text)) = (body["call"].as_str(), body["text"].as_str()) {
                    if body["implicit"].as_bool() == Some(true) {
                        let turn = body["turn"].as_u64();
                        if let Some(s) = conv.steps.iter_mut().rev().find(|s| Some(s.turn) == turn)
                        {
                            s.notice = Some(text.to_string());
                        }
                    } else {
                        results.insert(call.to_string(), text.to_string());
                    }
                }
            }
            "ctx.compacted" => {
                conv.compactions += 1;
                for x in body["elided"].as_array().into_iter().flatten() {
                    if let (Some(c), Some(p)) = (x["call"].as_str(), x["pointer"].as_str()) {
                        elided.insert(c.to_string(), p.to_string());
                    }
                }
                if let Some(cut) = body["cut"].as_u64() {
                    conv.cut = conv.cut.max(cut as usize);
                }
                if let Some(d) = body["digest"].as_str() {
                    conv.digest = Some(d.to_string());
                }
            }
            _ => {}
        }
    }
    for s in &mut conv.steps {
        for c in &mut s.calls {
            c.result = results.remove(&c.id);
            c.elided = elided.remove(&c.id);
        }
    }
    conv
}

pub struct Prefix {
    pub harness: String,
    pub brief: String,
}

pub fn render(prefix: &Prefix, conv: &Conversation, ledger: &str) -> Vec<Msg> {
    let mut out = vec![
        Msg::System(prefix.harness.clone()),
        Msg::System(prefix.brief.clone()),
        Msg::User(BEGIN.into()),
    ];
    if let Some(d) = &conv.digest {
        out.push(Msg::User(d.clone()));
    }
    for s in conv.steps.iter().skip(conv.cut) {
        out.push(Msg::Assistant {
            text: s.text.clone(),
            calls: s
                .calls
                .iter()
                .map(|c| ToolCall {
                    id: c.provider_id.clone(),
                    name: c.name.clone(),
                    arguments: if c.elided.is_some() {
                        elide_arguments(&c.arguments)
                    } else {
                        c.arguments.clone()
                    },
                })
                .collect(),
        });
        for c in &s.calls {
            let text = match (&c.elided, &c.result) {
                (Some(p), _) => p.clone(),
                (None, Some(r)) => r.clone(),
                // Only the last step can lack results, and it's rendered
                // after they arrive; this is the defensive case.
                (None, None) => "[kitsu: no result recorded for this call]".into(),
            };
            out.push(Msg::Tool {
                call_id: c.provider_id.clone(),
                text,
            });
        }
        if let Some(n) = &s.notice {
            out.push(Msg::User(n.clone()));
        }
    }
    out.push(Msg::User(ledger.to_string()));
    out
}

/// Large string arguments (file contents, edit bodies) shrink to a size
/// and hash after compaction; the call's shape stays.
fn elide_arguments(args: &str) -> String {
    let Ok(Value::Object(mut m)) = serde_json::from_str::<Value>(args) else {
        return args.to_string();
    };
    for v in m.values_mut() {
        if let Value::String(s) = v
            && s.len() > 400
        {
            *v = Value::String(format!(
                "<elided: {} bytes, sha {}>",
                s.len(),
                &crate::util::content_id(s.as_bytes())[..12]
            ));
        }
    }
    Value::Object(m).to_string()
}

/// Estimated tokens of a request, conservatively (bytes / 3).
pub fn estimate(msgs: &[Msg]) -> u64 {
    let bytes: usize = msgs
        .iter()
        .map(|m| match m {
            Msg::System(t) | Msg::User(t) => t.len(),
            Msg::Assistant { text, calls } => {
                text.as_ref().map_or(0, |t| t.len())
                    + calls
                        .iter()
                        .map(|c| c.name.len() + c.arguments.len() + 16)
                        .sum::<usize>()
            }
            Msg::Tool { text, .. } => text.len() + 16,
        })
        .sum();
    (bytes as u64).div_ceil(3)
}

/// Ids of every call, for tests and the digest.
pub fn call_ids(conv: &Conversation) -> BTreeSet<String> {
    conv.steps
        .iter()
        .flat_map(|s| s.calls.iter().map(|c| c.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn journal() -> Vec<Value> {
        vec![
            json!({ "kind": "model.response", "body": { "turn": 1, "text": "Reading.", "calls": [
                { "call": "r1/c1", "provider_id": "p1", "name": "read_file", "arguments": "{\"path\":\"a.py\"}" }
            ] } }),
            json!({ "kind": "tool.end", "body": { "call": "r1/c1", "text": "1\tprint()" } }),
            json!({ "kind": "model.response", "body": { "turn": 2, "calls": [
                { "call": "r1/c2", "provider_id": "p2", "name": "write_file", "arguments": "{\"path\":\"a.py\",\"content\":\"x\"}" }
            ] } }),
        ]
    }

    #[test]
    fn fold_pairs_results_and_leaves_the_rest_pending() {
        let c = fold(&journal());
        assert_eq!(c.turns(), 2);
        assert_eq!(c.calls_issued, 2);
        assert_eq!(c.steps[0].calls[0].result.as_deref(), Some("1\tprint()"));
        let p = c.pending();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, "r1/c2");
    }

    #[test]
    fn the_prefix_is_fixed_and_the_ledger_is_last() {
        let mut c = fold(&journal());
        c.set_result("r1/c2", "Wrote a.py".into());
        let p = Prefix {
            harness: "H".into(),
            brief: "B".into(),
        };
        let a = render(&p, &c, "ledger 1");
        let b = render(&p, &c, "ledger 2");
        assert_eq!(
            a[..a.len() - 1],
            b[..b.len() - 1],
            "only the ledger differs"
        );
        assert_eq!(a[0], Msg::System("H".into()));
        assert_eq!(a[1], Msg::System("B".into()));
        assert_eq!(a.last(), Some(&Msg::User("ledger 1".into())));
        assert!(matches!(&a[4], Msg::Tool { call_id, .. } if call_id == "p1"));
    }
}
