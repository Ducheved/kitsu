//! Retrieval eval: does the index find the file that answers a question?
//!
//!     cargo run --release --example retrieval [-- fixtures/retrieval/kitsu.toml]
//!
//! Runs every question in the fixture against this repository at HEAD with
//! the FTS index, and against the baseline an agent has without it: grep
//! for the query words and rank files by how many distinct words they
//! contain. Prints recall@5, recall@20 and MRR per kind of question, and
//! search latency. Ranks are over distinct files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use kitsu::git::Git;
use kitsu::index::Index;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Fixture {
    q: Vec<Question>,
}

#[derive(Deserialize)]
struct Question {
    kind: String,
    query: String,
    expect: Vec<String>,
}

#[derive(Default)]
struct Score {
    n: usize,
    at5: usize,
    at20: usize,
    rr: f64,
}

impl Score {
    fn add(&mut self, rank: Option<usize>) {
        self.n += 1;
        if let Some(r) = rank {
            self.at5 += usize::from(r <= 5);
            self.at20 += usize::from(r <= 20);
            self.rr += 1.0 / r as f64;
        }
    }

    fn json(&self) -> serde_json::Value {
        let n = self.n.max(1) as f64;
        json!({
            "n": self.n,
            "recall@5": (self.at5 as f64 / n * 100.0).round() / 100.0,
            "recall@20": (self.at20 as f64 / n * 100.0).round() / 100.0,
            "mrr": (self.rr / n * 100.0).round() / 100.0,
        })
    }
}

fn first_rank(paths: &[String], expect: &[String]) -> Option<usize> {
    let mut seen = BTreeSet::new();
    let mut rank = 0;
    for p in paths {
        if seen.insert(p.as_str()) {
            rank += 1;
            if expect.contains(p) {
                return Some(rank);
            }
        }
    }
    None
}

fn words(q: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "and", "the", "of", "to", "is", "it", "in", "on", "for", "what", "when", "how",
        "does", "with", "which", "by", "be", "are", "as", "at", "or", "this", "from", "where",
        "who", "why", "do",
    ];
    q.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty() && !STOP.contains(&w.as_str()))
        .collect()
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let fixture = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("fixtures/retrieval/kitsu.toml"));
    let f: Fixture = toml::from_str(&std::fs::read_to_string(&fixture).expect("fixture"))
        .expect("parse fixture");
    let git = Git::new(&root);
    let head = git.rev("HEAD").expect("HEAD");

    // Missing expected paths make a question unanswerable; say so.
    let tree: BTreeSet<String> = git
        .tree_blobs(&head, "")
        .expect("tree")
        .into_iter()
        .map(|b| b.path)
        .collect();
    for q in &f.q {
        for e in &q.expect {
            assert!(tree.contains(e), "fixture expects {e}, which isn't in HEAD");
        }
    }

    let dir = std::env::temp_dir().join(format!("kitsu-retrieval-{}", std::process::id()));
    let mut ix = Index::open(&dir.join("index.db")).expect("index");
    let t = Instant::now();
    let updated = ix.update(&git, &head).expect("update");
    let index_ms = t.elapsed().as_millis();

    // Baseline corpus: every text file, lowercased.
    let files: Vec<(String, String)> = git
        .files_at(&head, "")
        .expect("files")
        .into_iter()
        .filter(|(_, b)| !b[..b.len().min(8000)].contains(&0))
        .map(|(p, b)| (p, String::from_utf8_lossy(&b).to_lowercase()))
        .collect();

    let mut fts: BTreeMap<String, Score> = BTreeMap::new();
    let mut grep: BTreeMap<String, Score> = BTreeMap::new();
    let mut lat = Vec::new();
    let mut misses = Vec::new();
    for q in &f.q {
        let t = Instant::now();
        let hits = ix.search(&git, &q.query, 60).expect("search");
        lat.push(t.elapsed().as_secs_f64() * 1000.0);
        let paths: Vec<String> = hits.into_iter().map(|h| h.path).collect();
        let r = first_rank(&paths, &q.expect);
        if r.is_none_or(|r| r > 5) {
            let mut seen = BTreeSet::new();
            let first3: Vec<&String> = paths
                .iter()
                .filter(|p| seen.insert(p.as_str()))
                .take(3)
                .collect();
            misses.push(json!({ "query": q.query, "rank": r, "first3": first3 }));
        }
        for key in [q.kind.clone(), "all".into()] {
            fts.entry(key).or_default().add(r);
        }

        let ws = words(&q.query);
        let mut ranked: Vec<(usize, usize, &String)> = files
            .iter()
            .map(|(p, text)| {
                let distinct = ws.iter().filter(|w| text.contains(w.as_str())).count();
                let total: usize = ws.iter().map(|w| text.matches(w.as_str()).count()).sum();
                (distinct, total, p)
            })
            .filter(|(d, ..)| *d > 0)
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
        let paths: Vec<String> = ranked.into_iter().map(|(.., p)| p.clone()).collect();
        let r = first_rank(&paths, &q.expect);
        for key in [q.kind.clone(), "all".into()] {
            grep.entry(key).or_default().add(r);
        }
    }
    lat.sort_by(f64::total_cmp);
    let pct = |p: f64| lat[((lat.len() as f64 - 1.0) * p).round() as usize];
    let out = json!({
        "commit": &head[..12],
        "questions": f.q.len(),
        "index": { "files": updated.files, "indexed": updated.new_blobs, "skipped": updated.skipped, "ms": index_ms },
        "fts": fts.iter().map(|(k, s)| (k.clone(), s.json())).collect::<BTreeMap<_, _>>(),
        "grep_baseline": grep.iter().map(|(k, s)| (k.clone(), s.json())).collect::<BTreeMap<_, _>>(),
        "search_ms": { "p50": (pct(0.5) * 10.0).round() / 10.0, "p95": (pct(0.95) * 10.0).round() / 10.0 },
        "fts_misses_at_5": misses,
    });
    println!("{}", serde_json::to_string_pretty(&out).expect("json"));
    let _ = std::fs::remove_dir_all(&dir);
}
