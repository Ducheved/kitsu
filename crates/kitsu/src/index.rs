//! A local full-text index over the code at a commit.
//!
//! Git is the authority and this is a cache that knows how to invalidate
//! itself: chunks are keyed by blob id, so re-indexing a new commit reads
//! only blobs it hasn't seen, and deleting `index.db` loses nothing.
//!
//! Lexical on purpose (SQLite FTS5, BM25). Identifiers are also indexed
//! split into words (`retryBudget`, `retry_budget` -> `retry budget`) and
//! so are path parts, so both "RetryBudget" and "retry budget" find it. No
//! embeddings until the retrieval eval shows they'd find what this misses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::git::{Git, TreeBlob};

const SCHEMA_VERSION: i64 = 2;

/// Lines per chunk, and how far each chunk starts after the previous one.
/// The overlap keeps a function that straddles a boundary findable whole.
pub const CHUNK_LINES: usize = 60;
const CHUNK_STEP: usize = 50;
/// Bigger files are almost always generated or data.
const MAX_BYTES: u64 = 1 << 20;
/// Blobs read per `cat-file` call, to bound memory on the first index.
const READ_BATCH: usize = 512;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Which blobs have been looked at, and why one wasn't indexed.
CREATE TABLE IF NOT EXISTS blobs (
    oid     TEXT PRIMARY KEY,
    lines   INTEGER NOT NULL,
    skipped TEXT
);
-- The tree that was indexed last: path -> blob.
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    oid  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS files_by_oid ON files(oid);
-- Where each indexed chunk is. The text itself isn't stored: git has it,
-- and storing it doubled the index (202 MB for an 8.6k-file repo).
CREATE TABLE IF NOT EXISTS spans (
    id    INTEGER PRIMARY KEY,
    oid   TEXT NOT NULL,
    start INTEGER NOT NULL,
    end   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS spans_by_oid ON spans(oid);
CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
    text,
    terms,
    content = '',
    contentless_delete = 1,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
"#;

pub struct Index {
    conn: Connection,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Updated {
    pub rev: String,
    pub files: usize,
    /// Blobs read and chunked in this update (the rest were already known).
    pub new_blobs: usize,
    pub skipped: usize,
    pub chunks_added: usize,
    pub blobs_dropped: usize,
    pub ms: u128,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Hit {
    pub path: String,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    pub score: f64,
    /// The lines of the chunk that contain a query term, at most a few.
    pub lines: Vec<(usize, String)>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir.display().to_string(), e))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            // A cache: on any other version, start over.
            conn.execute_batch(
                "DROP TABLE IF EXISTS chunks; DROP TABLE IF EXISTS spans; DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS blobs; DROP TABLE IF EXISTS meta;",
            )?;
            // Dropped tables leave their pages behind; give them back.
            conn.execute_batch("VACUUM;")?;
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Index { conn })
    }

    pub fn indexed_rev(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'rev'", [], |r| r.get(0))
            .optional()?)
    }

    /// Make the index describe `rev`. Cheap when little changed.
    pub fn update(&mut self, git: &Git, rev: &str) -> Result<Updated> {
        let started = Instant::now();
        let commit = git.rev(rev)?;
        if self.indexed_rev()?.as_deref() == Some(commit.as_str()) {
            let files: i64 = self
                .conn
                .query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
            return Ok(Updated {
                rev: commit,
                files: files as usize,
                ms: started.elapsed().as_millis(),
                ..Updated::default()
            });
        }
        let tree = git.tree_blobs(&commit, "")?;
        let known: BTreeSet<String> = {
            let mut st = self.conn.prepare("SELECT oid FROM blobs")?;
            st.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut todo: Vec<&TreeBlob> = Vec::new();
        let mut seen = BTreeSet::new();
        for b in &tree {
            if !known.contains(&b.oid) && seen.insert(b.oid.as_str()) {
                todo.push(b);
            }
        }
        let mut out = Updated {
            rev: commit.clone(),
            files: tree.len(),
            new_blobs: todo.len(),
            ..Updated::default()
        };

        let tx =
            rusqlite::Transaction::new(&mut self.conn, rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut add_blob = tx.prepare_cached(
                "INSERT OR REPLACE INTO blobs (oid, lines, skipped) VALUES (?1, ?2, ?3)",
            )?;
            let mut add_span =
                tx.prepare_cached("INSERT INTO spans (oid, start, end) VALUES (?1, ?2, ?3)")?;
            let mut add_chunk =
                tx.prepare_cached("INSERT INTO chunks (rowid, text, terms) VALUES (?1, ?2, ?3)")?;
            let (skip_now, read): (Vec<&TreeBlob>, Vec<&TreeBlob>) = todo
                .iter()
                .partition(|b| skip_reason(&b.path, b.size, None).is_some());
            for b in skip_now {
                add_blob.execute(params![b.oid, 0, skip_reason(&b.path, b.size, None)])?;
                out.skipped += 1;
            }
            for batch in read.chunks(READ_BATCH) {
                let contents = git.read_blobs(batch.iter().map(|b| b.oid.as_str()))?;
                for (b, bytes) in batch.iter().zip(contents) {
                    if let Some(why) = skip_reason(&b.path, b.size, Some(&bytes)) {
                        add_blob.execute(params![b.oid, 0, why])?;
                        out.skipped += 1;
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    let lines: Vec<&str> = text.lines().collect();
                    for (start, end) in chunk_ranges(lines.len()) {
                        let body = lines[start..end].join("\n");
                        add_span.execute(params![b.oid, (start + 1) as i64, end as i64])?;
                        add_chunk.execute(params![
                            tx.last_insert_rowid(),
                            body,
                            split_terms(&body)
                        ])?;
                        out.chunks_added += 1;
                    }
                    add_blob.execute(params![b.oid, lines.len() as i64, Option::<String>::None])?;
                }
            }
        }
        tx.execute("DELETE FROM files", [])?;
        {
            let mut add_file =
                tx.prepare_cached("INSERT OR REPLACE INTO files (path, oid) VALUES (?1, ?2)")?;
            for b in &tree {
                add_file.execute(params![b.path, b.oid])?;
            }
        }
        // Blobs no longer in the tree: drop their chunks so the index stays
        // the size of one tree, not of the whole history.
        out.blobs_dropped = tx.execute(
            "DELETE FROM blobs WHERE oid NOT IN (SELECT oid FROM files)",
            [],
        )?;
        tx.execute(
            "DELETE FROM chunks WHERE rowid IN (SELECT id FROM spans WHERE oid NOT IN (SELECT oid FROM files))",
            [],
        )?;
        tx.execute(
            "DELETE FROM spans WHERE oid NOT IN (SELECT oid FROM files)",
            [],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('rev', ?1)",
            [&commit],
        )?;
        tx.commit()?;
        out.ms = started.elapsed().as_millis();
        Ok(out)
    }

    /// Chunks that best match `query`, best first. Path terms count, so
    /// "runner stop" prefers runner.rs.
    /// `git` reads back the lines of the hits (the index doesn't keep text).
    pub fn search(&self, git: &Git, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let words = query_words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let fts = words
            .iter()
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut st = self.conn.prepare_cached(
            "SELECT s.oid, s.start, s.end, m.rank FROM
               (SELECT rowid, bm25(chunks, 1.0, 0.6) AS rank FROM chunks WHERE chunks MATCH ?1 ORDER BY rank LIMIT ?2) m
             JOIN spans s ON s.id = m.rowid ORDER BY m.rank",
        )?;
        // Overfetch: a blob shared by several paths expands to several hits,
        // and path boosts reorder.
        let rows: Vec<(String, usize, usize, f64)> = st
            .query_map(params![fts, (limit * 4).max(20) as i64], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, i64>(1)? as usize,
                    r.get::<_, i64>(2)? as usize,
                    r.get(3)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let oids: Vec<String> = rows
            .iter()
            .map(|r| r.0.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let texts: BTreeMap<String, Vec<u8>> = oids
            .iter()
            .cloned()
            .zip(git.read_blobs(oids.iter().map(String::as_str))?)
            .collect();
        let mut paths_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let mut p = self
                .conn
                .prepare_cached("SELECT path FROM files WHERE oid = ?1 ORDER BY path")?;
            for (oid, ..) in &rows {
                if !paths_of.contains_key(oid) {
                    let v: Vec<String> = p
                        .query_map([oid], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    paths_of.insert(oid.clone(), v);
                }
            }
        }
        let stems: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let mut hits = Vec::new();
        for (oid, start, end, rank) in rows {
            let text =
                String::from_utf8_lossy(texts.get(&oid).map(Vec::as_slice).unwrap_or_default());
            let chunk: Vec<&str> = text.lines().skip(start - 1).take(end + 1 - start).collect();
            for path in paths_of.get(&oid).into_iter().flatten() {
                // bm25() is negative, lower is better; flip it and add a
                // bonus per query word found in the path.
                let path_words: Vec<String> = path
                    .split(|c: char| !c.is_alphanumeric())
                    .flat_map(|p| std::iter::once(p.to_lowercase()).chain(split_ident(p)))
                    .collect();
                let bonus = stems
                    .iter()
                    .filter(|w| path_words.iter().any(|p| p == *w))
                    .count() as f64;
                let lines = chunk
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| {
                        let l = l.to_lowercase();
                        stems.iter().any(|w| l.contains(w.as_str()))
                    })
                    .take(3)
                    .map(|(i, l)| (start + i, l.trim_end().chars().take(200).collect()))
                    .collect();
                hits.push(Hit {
                    path: path.clone(),
                    start,
                    end,
                    score: -rank + bonus * 2.0,
                    lines,
                });
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.path.cmp(&b.path))
                .then(a.start.cmp(&b.start))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

fn skip_reason(path: &str, size: u64, bytes: Option<&[u8]>) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    const GENERATED: &[&str] = &[
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "poetry.lock",
        "go.sum",
    ];
    if GENERATED.contains(&name) || name.ends_with(".min.js") || name.ends_with(".map") {
        return Some("lockfile or generated".into());
    }
    if size > MAX_BYTES {
        return Some(format!("larger than {} KiB", MAX_BYTES / 1024));
    }
    if let Some(b) = bytes
        && b[..b.len().min(8000)].contains(&0)
    {
        return Some("binary".into());
    }
    None
}

fn chunk_ranges(lines: usize) -> Vec<(usize, usize)> {
    if lines == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + CHUNK_LINES).min(lines);
        out.push((start, end));
        if end == lines {
            break;
        }
        start += CHUNK_STEP;
    }
    out
}

/// Identifier parts and path parts, lowercased: what a person types when
/// they don't remember the exact name.
pub fn split_terms(text: &str) -> String {
    let mut out = String::new();
    for ident in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        let parts = split_ident(ident);
        if parts.len() > 1 {
            for p in parts {
                out.push_str(&p);
                out.push(' ');
            }
        }
    }
    out
}

fn split_ident(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for piece in ident.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<char> = piece.chars().collect();
        let mut cur = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = i > 0
                && c.is_uppercase()
                && (chars[i - 1].is_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars.get(i + 1).is_some_and(|n| n.is_lowercase())
                        && chars[i - 1].is_uppercase());
            if boundary && !cur.is_empty() {
                parts.push(cur.to_lowercase());
                cur.clear();
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            parts.push(cur.to_lowercase());
        }
    }
    parts
}

/// Query words minus the ones that match everything. Identifiers in the
/// query also contribute their parts.
fn query_words(q: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "do", "does", "for", "from", "how", "in",
        "is", "it", "of", "on", "or", "the", "this", "to", "what", "when", "where", "which", "who",
        "why", "with",
    ];
    let mut out: Vec<String> = Vec::new();
    for w in q
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
    {
        let lw = w.to_lowercase();
        if STOP.contains(&lw.as_str()) {
            continue;
        }
        if !out.contains(&lw) {
            out.push(lw);
        }
        for p in split_ident(w) {
            if p.len() > 1 && !STOP.contains(&p.as_str()) && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;

    #[test]
    fn identifiers_split_the_way_people_search() {
        assert_eq!(split_ident("retryBudget"), ["retry", "budget"]);
        assert_eq!(split_ident("retry_budget"), ["retry", "budget"]);
        assert_eq!(split_ident("HTTPServer"), ["http", "server"]);
        assert_eq!(split_ident("parseV2Header"), ["parse", "v2", "header"]);
        assert_eq!(split_terms("fn maxAttempts() {}"), "max attempts ");
        assert_eq!(
            query_words("where is the retryBudget"),
            ["retrybudget", "retry", "budget"]
        );
    }

    #[test]
    fn chunks_overlap_and_cover_everything() {
        assert_eq!(chunk_ranges(0), vec![]);
        assert_eq!(chunk_ranges(10), vec![(0, 10)]);
        assert_eq!(chunk_ranges(120), vec![(0, 60), (50, 110), (100, 120)]);
    }

    fn index_for(repo: &TempRepo) -> Index {
        Index::open(&repo.root.join(".git/kitsu/index.db")).expect("open")
    }

    #[test]
    fn finds_code_by_split_names_and_updates_by_blob() {
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let repo = TempRepo::new(&[
            (
                "src/client.rs",
                "pub fn call_with_retry(maxAttempts: u32) {}\n",
            ),
            (
                "src/copy.rs",
                "pub fn call_with_retry(maxAttempts: u32) {}\n",
            ),
            ("src/long.rs", &long),
            ("Cargo.lock", "retry retry retry\n"),
            ("img.bin", "\u{0}\u{1}retry"),
        ]);
        let git = repo.git();
        let mut ix = index_for(&repo);
        let u = ix.update(&git, "HEAD").expect("update");
        assert_eq!(u.files, 5);
        assert_eq!(
            u.new_blobs, 4,
            "identical files share a blob and are read once"
        );
        assert_eq!(u.skipped, 2, "lockfile and binary");

        let hits = ix.search(&git, "max attempts", 10).expect("search");
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(
            paths,
            ["src/client.rs", "src/copy.rs"],
            "one blob, both paths"
        );
        assert_eq!(hits[0].lines[0].0, 1);
        assert!(
            ix.search(&git, "retry", 10)
                .expect("s")
                .iter()
                .all(|h| h.path != "Cargo.lock")
        );
        let deep = ix.search(&git, "line 150", 10).expect("s");
        assert!(
            deep.iter()
                .any(|h| h.path == "src/long.rs" && h.start <= 151 && h.end >= 151),
            "{deep:?}"
        );

        // Unchanged commit: nothing to do.
        assert_eq!(ix.update(&git, "HEAD").expect("again").new_blobs, 0);

        // One file changes: one blob read, the old one's chunks dropped.
        repo.write("src/client.rs", "pub fn call_once() {}\n");
        repo.commit_all("change");
        let u = ix.update(&git, "HEAD").expect("update 2");
        assert_eq!(u.new_blobs, 1);
        let hits = ix.search(&git, "max attempts", 10).expect("search");
        assert_eq!(
            hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            ["src/copy.rs"]
        );
        assert_eq!(
            ix.search(&git, "call once", 10).expect("s")[0].path,
            "src/client.rs"
        );
    }

    #[test]
    fn path_words_rank_the_file_they_name_first() {
        let repo = TempRepo::new(&[
            ("src/runner.rs", "fn tick() { stop(); }\n"),
            ("src/other.rs", "fn stop() {} // stop stop\n"),
        ]);
        let mut ix = index_for(&repo);
        ix.update(&repo.git(), "HEAD").expect("update");
        let hits = ix.search(&repo.git(), "runner stop", 5).expect("search");
        assert_eq!(hits[0].path, "src/runner.rs", "{hits:?}");
    }
}
