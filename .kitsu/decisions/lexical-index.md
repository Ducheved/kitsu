+++
title = "A lexical index, measured against grep; no embeddings until they beat it"
scope = ["crates/kitsu/src/index.rs", "crates/kitsu/src/brief.rs"]
rejected = [
  "Embeddings now: every miss in the eval is a ranking miss (the answer is in the top 20), which a reranker addresses more cheaply than a second retrieval system",
  "Storing chunk text in the index: 5x the size (202 MB vs 38 MB on openai/codex) for text git already has",
]
+++
`kitsu search` is SQLite FTS5 with BM25 over 60-line chunks, identifiers
split into words, path words as a boost, keyed by git blob id.

Measured with `cargo run --release --example retrieval` on 34 questions
about this repository with known answers (fixtures/retrieval/kitsu.toml;
written by someone who knows the code, so indicative, not a benchmark):

| | recall@5 | recall@20 | MRR |
|---|---|---|---|
| index, questions in words (28) | 0.86 | 1.00 | 0.67 |
| grep for the words, rank by distinct words (28) | 0.71 | 0.96 | 0.54 |
| both, questions naming an identifier (6) | 1.00 | 1.00 | 0.92 / 0.81 |

Search takes about 20 ms, most of it starting `git cat-file` to read back
hit lines.

Gates for what comes next:
- Embeddings: only if a held-out question set (not written by the index's
  author) shows at least 10 points of recall@5 over the index.
- A reranker (typed judgment): only if it moves MRR on the same set by at
  least 0.1 at a cost below 1 cent per query.
- Relevant code in briefs: only with a budget and a "left out, search for
  it" line, and only once a real agent trial shows fewer exploratory reads.

Known ranking noise: the five locale files repeat the same English keys,
so UI-concept questions pull them up.
