//! Noticing a model going in circles, from structure only.
//!
//! A call counts as a repeat when the same tool gets the same arguments
//! (keys in any order) and the worktree hasn't changed in between: then the
//! result can only be the same. Three of those within the last eight calls
//! is a signal. Rereading prose for signs of "giving up" is what other
//! harnesses do; it misfires both ways, so this doesn't.

use std::collections::VecDeque;

use serde_json::Value;

use crate::util::content_id;

const WINDOW: usize = 8;
const REPEATS: usize = 3;

pub const WARNING: &str = "[kitsu: this is the third identical call with no change to your files in between, so the result is the same. Change approach, or call finish with outcome blocked and say what you need.]";

#[derive(Default)]
pub struct Detector {
    recent: VecDeque<(String, String)>,
    pub signals: u32,
}

impl Detector {
    /// Record a call and the worktree tree after it. Returns the signal
    /// count when this call is a signal (1: warn, 2: stop).
    pub fn observe(&mut self, name: &str, arguments: &str, tree: &str) -> Option<u32> {
        let fp = fingerprint(name, arguments);
        self.recent.push_back((fp.clone(), tree.to_string()));
        while self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
        let same: Vec<&(String, String)> = self.recent.iter().filter(|(f, _)| *f == fp).collect();
        if same.len() >= REPEATS && same.iter().all(|(_, t)| t == tree) {
            self.signals += 1;
            // A signal consumes its repeats, so one loop isn't counted twice.
            self.recent.retain(|(f, _)| *f != fp);
            return Some(self.signals);
        }
        None
    }
}

fn fingerprint(name: &str, arguments: &str) -> String {
    let canonical = match serde_json::from_str::<Value>(arguments) {
        Ok(v) => canonical(&v),
        Err(_) => arguments.to_string(),
    };
    content_id(format!("{name}\0{canonical}").as_bytes())
}

/// JSON with object keys sorted, so `{"a":1,"b":2}` and `{"b":2,"a":1}`
/// are the same call.
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical(&m[k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_identical_calls_on_an_unchanged_tree_signal_once() {
        let mut d = Detector::default();
        assert_eq!(
            d.observe("read_file", r#"{"path":"a","start_line":1}"#, "t1"),
            None
        );
        assert_eq!(d.observe("grep", r#"{"pattern":"x"}"#, "t1"), None);
        assert_eq!(
            d.observe("read_file", r#"{"start_line":1,"path":"a"}"#, "t1"),
            None
        );
        assert_eq!(
            d.observe("read_file", r#"{"path":"a","start_line":1}"#, "t1"),
            Some(1)
        );
        // The repeats were consumed: the next identical call isn't a second signal yet.
        assert_eq!(
            d.observe("read_file", r#"{"path":"a","start_line":1}"#, "t1"),
            None
        );
    }

    #[test]
    fn repeats_with_changes_in_between_are_progress() {
        let mut d = Detector::default();
        for (i, tree) in ["t1", "t2", "t3", "t4"].iter().enumerate() {
            assert_eq!(
                d.observe("run_check", r#"{"name":"test"}"#, tree),
                None,
                "{i}"
            );
        }
    }

    #[test]
    fn repeats_far_apart_are_not_a_loop() {
        let mut d = Detector::default();
        d.observe("read_file", r#"{"path":"a"}"#, "t");
        for i in 0..8 {
            d.observe("read_file", &format!(r#"{{"path":"f{i}"}}"#), "t");
        }
        d.observe("read_file", r#"{"path":"a"}"#, "t");
        assert_eq!(d.observe("read_file", r#"{"path":"a"}"#, "t"), None);
    }
}
