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
    /// What Kitsu answered to a reply with no tool calls: a nudge, a note
    /// that it was cut off, or, when it's taken as `finish`, finish's
    /// answer. Rendered as a user message after the step.
    pub notice: Option<String>,
    /// Opaque provider state for this reply (`provider::Reply::replay`),
    /// carried through for the provider that wrote it.
    pub replay: Option<Value>,
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
    let mut notes: BTreeMap<String, String> = BTreeMap::new();
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
                    replay: body.get("replay").filter(|r| !r.is_null()).cloned(),
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
            "loop.signal" => {
                let note = body["note"].as_str();
                if let (Some(c), Some(n)) = (body["call"].as_str(), note) {
                    notes
                        .entry(c.to_string())
                        .and_modify(|x| *x = format!("{x}\n{n}"))
                        .or_insert_with(|| n.to_string());
                } else if let (Some(turn), Some(n)) = (body["turn"].as_u64(), note)
                    && let Some(s) = conv.steps.iter_mut().rev().find(|s| s.turn == turn)
                {
                    // Said about a reply as a whole (no call, cut off).
                    s.notice = Some(n.to_string());
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
            c.result = results.remove(&c.id).map(|r| match notes.remove(&c.id) {
                Some(n) => format!("{r}\n{n}"),
                None => r,
            });
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
            replay: s.replay.clone(),
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

/// Estimated tokens of a request, conservatively (bytes / 3). `extra` is
/// what goes with every request besides the messages: the tool list.
pub fn estimate(msgs: &[Msg], extra: usize) -> u64 {
    let bytes: usize = msgs.iter().map(msg_bytes).sum::<usize>() + extra;
    (bytes as u64).div_ceil(3)
}

fn msg_bytes(m: &Msg) -> usize {
    match m {
        Msg::System(t) | Msg::User(t) => t.len() + 8,
        Msg::Assistant { text, calls, .. } => {
            text.as_ref().map_or(0, |t| t.len())
                + calls
                    .iter()
                    .map(|c| c.name.len() + c.arguments.len() + 24)
                    .sum::<usize>()
                + 8
        }
        Msg::Tool { text, .. } => text.len() + 24,
    }
}

/// What a compaction changes: results replaced by pointers, and possibly
/// the oldest steps replaced by a digest.
#[derive(Debug, Clone, PartialEq)]
pub struct Compaction {
    pub elided: Vec<(String, String)>,
    pub cut: usize,
    pub digest: Option<String>,
    pub before: u64,
    pub after: u64,
}

impl Conversation {
    pub fn apply(&mut self, c: &Compaction) {
        self.compactions += 1;
        for (call, pointer) in &c.elided {
            for s in &mut self.steps {
                for x in &mut s.calls {
                    if &x.id == call {
                        x.elided = Some(pointer.clone());
                    }
                }
            }
        }
        self.cut = self.cut.max(c.cut);
        if c.digest.is_some() {
            self.digest = c.digest.clone();
        }
    }
}

/// Shrink the conversation toward `target` tokens. The result may still be
/// over it (`after > target`) when even the newest steps are that big; the
/// caller decides what that means.
///
/// Never touches the prefix or the ledger, never splits a step (a model
/// message and its tool results), and keeps the newest steps verbatim: up to
/// four, within `tail` tokens, at least one. First, older tool outputs (and
/// large arguments) become pointers; that's lossless, since every output is
/// in the journal and the model can call the tool again. If that isn't
/// enough, the older steps are replaced by a digest regenerated from all of
/// them every time, so it never becomes a summary of a summary.
pub fn compact(
    prefix: &Prefix,
    conv: &Conversation,
    ledger: &str,
    extra: usize,
    target: u64,
    tail: u64,
) -> Compaction {
    let size = |c: &Conversation| estimate(&render(prefix, c, ledger), extra);
    let before = size(conv);
    let k = conv.compactions + 1;
    let n = conv.steps.len();
    let mut tail_start = n;
    let mut acc = 0u64;
    while tail_start > conv.cut && n - tail_start < 4 {
        let s = &conv.steps[tail_start - 1];
        let t = step_tokens(s);
        if tail_start < n && acc + t > tail {
            break;
        }
        acc += t;
        tail_start -= 1;
    }
    let mut work = conv.clone();
    let mut elided = Vec::new();
    'stage1: for i in conv.cut..tail_start {
        for j in 0..work.steps[i].calls.len() {
            let c = work.steps[i].calls[j].clone();
            let big = c.result.as_ref().map_or(0, |r| r.len()) > 200 || c.arguments.len() > 400;
            if c.elided.is_some() || !big {
                continue;
            }
            let pointer = format!(
                "[kitsu: output of {} elided at compaction {k} ({} bytes), call {}. Call the tool again if you need it.]",
                describe(&c),
                c.result.as_ref().map_or(0, |r| r.len()),
                c.id
            );
            work.steps[i].calls[j].elided = Some(pointer.clone());
            elided.push((c.id.clone(), pointer));
            if size(&work) <= target {
                break 'stage1;
            }
        }
    }
    let mut cut = conv.cut;
    let mut digest = None;
    if size(&work) > target && tail_start > conv.cut {
        cut = tail_start;
        let d = make_digest(&work.steps[..cut], k);
        work.cut = cut;
        work.digest = Some(d.clone());
        digest = Some(d);
    }
    let after = size(&work);
    Compaction {
        elided,
        cut,
        digest,
        before,
        after,
    }
}

fn step_tokens(s: &Step) -> u64 {
    let bytes = s.text.as_ref().map_or(0, |t| t.len())
        + s.calls
            .iter()
            .map(|c| c.arguments.len() + c.result.as_ref().map_or(0, |r| r.len()) + 48)
            .sum::<usize>();
    (bytes as u64).div_ceil(3)
}

fn describe(c: &Call) -> String {
    let v: Value = serde_json::from_str(&c.arguments).unwrap_or(Value::Null);
    let arg = ["path", "command", "name", "pattern", "query"]
        .iter()
        .find_map(|k| v[*k].as_str())
        .map(|a| a.chars().take(80).collect::<String>());
    match arg {
        Some(a) => format!("{} {a}", c.name),
        None => c.name.clone(),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}

/// A record of the steps before the cut, built from what was recorded, not
/// written by a model.
fn make_digest(steps: &[Step], k: u64) -> String {
    let first = steps.first().map_or(0, |s| s.turn);
    let last = steps.last().map_or(0, |s| s.turn);
    let mut written: BTreeMap<String, u64> = BTreeMap::new();
    let mut read: BTreeMap<String, u64> = BTreeMap::new();
    let mut commands = Vec::new();
    let mut checks = Vec::new();
    let mut other = Vec::new();
    for s in steps {
        for c in &s.calls {
            let v: Value = serde_json::from_str(&c.arguments).unwrap_or(Value::Null);
            let path = v["path"].as_str().unwrap_or("").to_string();
            let res = c.result.as_deref().unwrap_or("(no result)");
            match c.name.as_str() {
                "write_file" | "edit_file" => {
                    written.insert(path, s.turn);
                }
                "read_file" => {
                    read.insert(path, s.turn);
                }
                "shell" => commands.push(format!(
                    "- turn {}: `{}` → {}",
                    s.turn,
                    v["command"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(120)
                        .collect::<String>(),
                    first_line(res)
                )),
                "run_check" | "finish" => checks.push(format!(
                    "- turn {}: {} → {}",
                    s.turn,
                    describe(c),
                    first_line(res)
                )),
                _ => other.push(format!("{} (turn {})", describe(c), s.turn)),
            }
        }
    }
    let mut out = format!(
        "[Kitsu journal record for turns {first}-{last} (compaction {k}). Generated from recorded events, not by a model. The state message at the end is current; this is history.]\n"
    );
    let list = |m: &BTreeMap<String, u64>| {
        m.iter()
            .map(|(p, t)| format!("{p} (turn {t})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !written.is_empty() {
        out.push_str(&format!("Files you wrote or edited: {}\n", list(&written)));
    }
    if !read.is_empty() {
        out.push_str(&format!("Files you read: {}\n", list(&read)));
    }
    if !commands.is_empty() {
        out.push_str("Commands:\n");
        for c in commands.iter().rev().take(20).rev() {
            out.push_str(c);
            out.push('\n');
        }
    }
    if !checks.is_empty() {
        out.push_str("Checks and finish:\n");
        for c in checks.iter().rev().take(20).rev() {
            out.push_str(c);
            out.push('\n');
        }
    }
    if !other.is_empty() {
        out.push_str(&format!(
            "Other calls: {}\n",
            other
                .iter()
                .rev()
                .take(30)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut words = String::new();
    for s in steps.iter().rev() {
        let Some(t) = s.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let t: String = t.trim().chars().take(1000).collect();
        let line = format!("- turn {}: {t}\n", s.turn);
        if words.len() + line.len() > 12_000 {
            break;
        }
        words.push_str(&line);
    }
    if !words.is_empty() {
        out.push_str("Your own words in those turns, newest first:\n");
        out.push_str(&words);
    }
    out
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

    fn big_conversation(steps: usize) -> Conversation {
        let mut entries = Vec::new();
        for i in 1..=steps {
            entries.push(json!({ "kind": "model.response", "body": { "turn": i, "text": format!("step {i}"), "calls": [
                { "call": format!("r1/c{i}"), "provider_id": format!("p{i}"), "name": "read_file", "arguments": format!("{{\"path\":\"f{i}.py\"}}") }
            ] } }));
            entries.push(json!({ "kind": "tool.end", "body": { "call": format!("r1/c{i}"), "text": "x".repeat(3000) } }));
        }
        fold(&entries)
    }

    #[test]
    fn compaction_elides_oldest_first_and_keeps_prefix_tail_and_step_pairs() {
        let p = Prefix {
            harness: "H".repeat(900),
            brief: "B".repeat(900),
        };
        let conv = big_conversation(8);
        let full = estimate(&render(&p, &conv, "L"), 0);
        let c = compact(&p, &conv, "L", 0, full - 1500, 2000);
        // Oldest results went first; the newest step is verbatim.
        assert_eq!(c.elided.first().map(|e| e.0.as_str()), Some("r1/c1"));
        assert!(c.elided.iter().all(|e| e.0 != "r1/c8"));
        assert!(c.digest.is_none(), "pointers were enough");
        let mut after = conv.clone();
        after.apply(&c);
        let msgs = render(&p, &after, "L");
        let before = render(&p, &conv, "L");
        assert_eq!(msgs[..3], before[..3], "the prefix never changes");
        assert_eq!(msgs.last(), before.last(), "the ledger is last");
        // Every assistant tool call is still followed by its result.
        assert_eq!(msgs.len(), before.len());
        assert!(
            matches!(&msgs[4], Msg::Tool { text, .. } if text.starts_with("[kitsu: output of read_file f1.py elided at compaction 1"))
        );
        assert!(
            matches!(msgs.iter().rev().nth(1), Some(Msg::Tool { text, .. }) if text.len() == 3000)
        );
        assert!(c.after <= full - 1500 && c.before == full);
    }

    #[test]
    fn when_pointers_arent_enough_old_steps_become_a_digest_of_the_journal() {
        let p = Prefix {
            harness: "H".into(),
            brief: "B".into(),
        };
        let conv = big_conversation(8);
        let c = compact(&p, &conv, "L", 0, 1300, 1100);
        assert!(c.after <= 1300);
        assert_eq!(c.cut, 7, "all but the tail step");
        let d = c.digest.clone().expect("digest");
        assert!(
            d.starts_with("[Kitsu journal record for turns 1-7 (compaction 1)"),
            "{d}"
        );
        assert!(d.contains("Files you read: f1.py (turn 1)"), "{d}");
        assert!(d.contains("- turn 7: step 7"), "{d}");
        let mut after = conv.clone();
        after.apply(&c);
        let msgs = render(&p, &after, "L");
        assert_eq!(msgs[3], Msg::User(d));
        assert!(matches!(&msgs[4], Msg::Assistant { text: Some(t), .. } if t == "step 8"));
        // The same journal gives the same compaction.
        assert_eq!(compact(&p, &conv, "L", 0, 1300, 1100), c);
        // A target nothing can reach: it does what it can and says so.
        let hard = compact(&p, &conv, "L", 0, 10, 1100);
        assert!(hard.after > 10 && hard.after < hard.before);
    }

    #[test]
    fn provider_state_rides_with_its_step_untouched() {
        let replay = json!({ "provider": "openai-responses", "items": [{ "type": "reasoning", "encrypted_content": "e" }] });
        let mut entries = journal();
        entries[0]["body"]["replay"] = replay.clone();
        let c = fold(&entries);
        assert_eq!(c.steps[0].replay.as_ref(), Some(&replay));
        assert_eq!(c.steps[1].replay, None);
        let p = Prefix {
            harness: "H".into(),
            brief: "B".into(),
        };
        let msgs = render(&p, &c, "L");
        assert!(matches!(&msgs[3], Msg::Assistant { replay: Some(r), .. } if *r == replay));
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
