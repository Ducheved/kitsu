//! "Since you left": what changed that a person should know about.
//!
//! Built from the event log after a sequence number the viewer last saw.
//! Deterministic counting and picking, no summarizing model: the point is
//! that you can trust it and click through to the source.

use std::collections::BTreeMap;

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
    pub text: String,
}

pub fn since(store: &Store, seq: i64) -> Result<Digest> {
    let events = store.events_after(seq, 5_000)?;
    let to_seq = events.last().map_or(seq, |e| e.seq);
    let mut finished: BTreeMap<String, String> = BTreeMap::new();
    let mut items = Vec::new();
    for e in &events {
        let run = e.run.clone();
        match e.kind.as_str() {
            "run.state" => {
                let to = e.body["to"].as_str().unwrap_or_default();
                let r = run.clone().unwrap_or_default();
                match to {
                    "finished" => {
                        finished
                            .insert(r, e.body["stop_reason"].as_str().unwrap_or("?").to_string());
                    }
                    "failed" => items.push(Item {
                        kind: "failed",
                        run,
                        text: format!(
                            "run {r} failed: {}",
                            e.body["detail"].as_str().unwrap_or("?")
                        ),
                    }),
                    "interrupted" => items.push(Item {
                        kind: "interrupted",
                        run,
                        text: format!("run {r} was interrupted; partial work kept"),
                    }),
                    _ => {}
                }
            }
            "ask.open" => {
                let title = e.body["request"]["title"].as_str().unwrap_or("a question");
                items.push(Item {
                    kind: "ask",
                    run,
                    text: format!("agent asked: {title}"),
                });
            }
            "check.done" if e.body["outcome"] != "pass" => {
                let check = e.body["check"].as_str().unwrap_or("?");
                items.push(Item {
                    kind: "check",
                    run,
                    text: format!(
                        "check {check} {}",
                        e.body["outcome"].as_str().unwrap_or("?")
                    ),
                });
            }
            "integration.state" if e.body["state"] == "applied" => {
                items.push(Item {
                    kind: "accepted",
                    run,
                    text: "change accepted".into(),
                });
            }
            "run.duplicate" => items.push(Item {
                kind: "protocol",
                run,
                text: "agent reported completion twice".into(),
            }),
            _ => {}
        }
    }
    for (r, reason) in finished {
        items.push(Item {
            kind: "finished",
            run: Some(r.clone()),
            text: format!("run {r} finished ({reason}), ready for review"),
        });
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
