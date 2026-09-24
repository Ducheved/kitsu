//! "Since you last looked": what changed that a person should know about.
//!
//! Built from the event log after a sequence number the viewer last saw.
//! Deterministic counting and picking, no summarizing model: the point is
//! that you can trust it and click through to the source.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::error::Result;
use crate::store::Store;

#[derive(Debug, Default, Serialize)]
pub struct Digest {
    pub from_seq: i64,
    pub to_seq: i64,
    /// One line per notable thing, most important first.
    pub items: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct Item {
    pub kind: &'static str,
    pub run: Option<String>,
    /// The task the run belongs to, so a UI can say which work this is.
    pub task: Option<String>,
    /// Reads after the task's name: "<task>: <text>".
    pub text: String,
}

fn stop_words(reason: &str) -> String {
    match reason {
        "end_turn" => "finished".into(),
        "cancelled" => "stopped when you asked".into(),
        "max_tokens" => "stopped: ran out of output tokens".into(),
        "max_turn_requests" => "stopped: hit its turn limit".into(),
        "refusal" => "refused to continue".into(),
        other => format!("stopped ({other})"),
    }
}

pub fn since(store: &Store, seq: i64) -> Result<Digest> {
    let events = store.events_after(seq, 5_000)?;
    let to_seq = events.last().map_or(seq, |e| e.seq);
    let mut task_of: HashMap<String, Option<String>> = HashMap::new();
    let mut task = |run: &Option<String>| -> Option<String> {
        let r = run.as_ref()?;
        task_of
            .entry(r.clone())
            .or_insert_with(|| store.run(r).ok().map(|x| x.task))
            .clone()
    };
    let mut finished: BTreeMap<String, String> = BTreeMap::new();
    let mut items: Vec<Item> = Vec::new();
    let push = |items: &mut Vec<Item>,
                kind: &'static str,
                run: Option<String>,
                task: Option<String>,
                text: String| {
        // The same fact recorded twice (a re-verify, a retried write) is one line.
        if !items.iter().any(|i| i.text == text && i.task == task) {
            items.push(Item {
                kind,
                run,
                task,
                text,
            });
        }
    };
    for e in &events {
        let run = e.run.clone();
        let t = task(&run);
        match e.kind.as_str() {
            "run.state" => match e.body["to"].as_str().unwrap_or_default() {
                "finished" => {
                    finished.insert(
                        run.clone().unwrap_or_default(),
                        e.body["stop_reason"]
                            .as_str()
                            .unwrap_or("end_turn")
                            .to_string(),
                    );
                }
                "failed" => push(
                    &mut items,
                    "failed",
                    run,
                    t,
                    format!(
                        "the agent failed: {}",
                        e.body["detail"].as_str().unwrap_or("unknown reason")
                    ),
                ),
                "interrupted" => push(
                    &mut items,
                    "interrupted",
                    run,
                    t,
                    "the run was interrupted; its partial work is kept".into(),
                ),
                _ => {}
            },
            "ask.open" => {
                let title = e.body["request"]["title"].as_str().unwrap_or("something");
                push(
                    &mut items,
                    "ask",
                    run,
                    t,
                    format!("the agent asks: {title}"),
                );
            }
            "check.done" if e.body["outcome"] != "pass" => {
                let check = e.body["check"].as_str().unwrap_or("?");
                let outcome = e.body["outcome"].as_str().unwrap_or("?");
                let text = if run.is_some() {
                    format!("check {check} {outcome}s on the agent's change")
                } else {
                    format!("check {check} {outcome}s on your checkout")
                };
                push(
                    &mut items,
                    "check",
                    run,
                    t,
                    text.replace("timeouts", "times out")
                        .replace("errors", "errored"),
                );
            }
            "integration.state" if e.body["state"] == "applied" => {
                push(&mut items, "accepted", run, t, "accepted".into())
            }
            "protocol.duplicate_response" => push(
                &mut items,
                "protocol",
                run,
                t,
                "the agent answered the same request twice (the second was ignored)".into(),
            ),
            _ => {}
        }
    }
    for (r, reason) in finished {
        let run = Some(r);
        let t = task(&run);
        let text = format!("{}, ready for review", stop_words(&reason));
        push(&mut items, "finished", run, t, text);
    }
    let rank = |k: &str| match k {
        "ask" => 0,
        "failed" | "interrupted" => 1,
        "check" => 2,
        "finished" => 3,
        _ => 4,
    };
    items.sort_by_key(|i| rank(i.kind));
    Ok(Digest {
        from_seq: seq,
        to_seq,
        items,
    })
}
