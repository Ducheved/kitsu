//! Memory notes: typed, anchored, and honest about going stale.
//!
//! A note in `.kitsu/memory/` names the paths it describes (`anchors`). Git
//! already knows when the note was last written and what those paths looked
//! like then, so the file carries no hashes: a note is stale when an anchor
//! changed between the last commit that touched the note and the commit
//! we're looking at. Editing the note re-anchors it, so editing is how you
//! confirm it still holds.
//!
//! Personal notes live in the user's config dir, apply to every repository,
//! and are never anchored: they're about a person, not the code.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::git::Git;
use crate::intent::{self, Memory, Source};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Freshness {
    /// Anchors unchanged since the note was last committed.
    Current,
    /// These anchored paths changed after the note was last committed.
    Stale { changed: Vec<String>, since: String },
    /// No anchors, so nothing in the code can make it stale.
    Unanchored,
    /// The note isn't in the commit we're looking at (written but not
    /// committed yet), so there's nothing to compare against.
    Uncommitted,
}

/// Freshness of every note at commit `at`. Two git calls for the history
/// plus one diff per distinct commit that last touched a note.
pub fn freshness<'a>(
    git: &Git,
    at: &str,
    notes: impl IntoIterator<Item = &'a Memory>,
) -> Result<BTreeMap<String, Freshness>> {
    let notes: Vec<&Memory> = notes.into_iter().collect();
    let mut out = BTreeMap::new();
    let anchored: Vec<&Memory> = notes
        .iter()
        .copied()
        .filter(|m| {
            if m.anchors.is_everything() {
                out.insert(m.id.clone(), Freshness::Unanchored);
                false
            } else {
                true
            }
        })
        .collect();
    if anchored.is_empty() {
        return Ok(out);
    }
    let written = last_commits(git, at)?;
    let mut changed_since: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for m in &anchored {
        let Some(c) = written.get(&m.source.path) else {
            out.insert(m.id.clone(), Freshness::Uncommitted);
            continue;
        };
        if !changed_since.contains_key(c.as_str()) {
            changed_since.insert(c, git.changed_paths(c, at)?);
        }
        let changed: Vec<String> = changed_since[c.as_str()]
            .iter()
            .filter(|p| m.anchors.contains(p))
            .cloned()
            .collect();
        let f = if changed.is_empty() {
            Freshness::Current
        } else {
            Freshness::Stale {
                changed,
                since: c.clone(),
            }
        };
        out.insert(m.id.clone(), f);
    }
    Ok(out)
}

/// For each file under `.kitsu/memory/` at `at`, the last commit that
/// changed it.
fn last_commits(git: &Git, at: &str) -> Result<BTreeMap<String, String>> {
    let dir = format!("{}/{}/", intent::DIR, intent::Kind::Memory.dir());
    let log = git.run([
        "log",
        "--format=%x01%H",
        "--name-only",
        "--no-renames",
        at,
        "--",
        &dir,
    ])?;
    let mut map = BTreeMap::new();
    let mut commit = "";
    for line in log.lines() {
        if let Some(c) = line.strip_prefix('\u{1}') {
            commit = c;
        } else if !line.is_empty() && !commit.is_empty() {
            map.entry(line.to_string())
                .or_insert_with(|| commit.to_string());
        }
    }
    Ok(map)
}

/// A person's own notes, and the ones that failed to parse (reported, not
/// dropped).
#[derive(Debug, Clone, Default)]
pub struct Personal {
    pub notes: Vec<Memory>,
    pub problems: Vec<(String, String)>,
}

/// Personal notes: `<config>/memory/*.md`.
pub fn personal(config_dir: &Path) -> Personal {
    let dir = config_dir.join("memory");
    let mut notes = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Personal { notes, problems };
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    for p in paths {
        let shown = p.display().to_string();
        let id = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = std::fs::read(&p)
            .map_err(|e| e.to_string())
            .and_then(|b| {
                String::from_utf8(b.clone())
                    .map(|t| (t, b))
                    .map_err(|_| "not valid UTF-8".to_string())
            })
            .and_then(|(text, bytes)| {
                let (front, body) = intent::split_front_matter(&text)?;
                let source = Source {
                    path: shown.clone(),
                    content_id: crate::util::content_id(&bytes),
                };
                intent::parse_memory(&id, front, body, source)
            });
        match parsed {
            Ok(mut m) if m.anchors.is_everything() => {
                m.id = format!("personal/{id}");
                notes.push(m);
            }
            Ok(_) => problems.push((
                shown,
                "personal notes can't have anchors: they aren't about one repository".into(),
            )),
            Err(e) => problems.push((shown, e)),
        }
    }
    Personal { notes, problems }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::TempRepo;
    use crate::intent::Intent;

    const NOTE: &str = "+++\ntitle = \"Upstream dedupes keys for 24h\"\nkind = \"fact\"\nscope = [\"payments.py\"]\nanchors = [\"payments.py\"]\n+++\nSeen in their docs.\n";

    fn notes_at(repo: &TempRepo, at: &str) -> (Intent, BTreeMap<String, Freshness>) {
        let git = repo.git();
        let intent = Intent::from_files(git.files_at(at, intent::DIR).expect("files"));
        let f = freshness(&git, at, intent.memory.values()).expect("freshness");
        (intent, f)
    }

    #[test]
    fn a_note_goes_stale_when_its_anchor_changes_and_editing_it_confirms_it() {
        let repo = TempRepo::new(&[
            ("payments.py", "def charge(): pass\n"),
            ("README.md", "hi\n"),
            (".kitsu/memory/dedupe.md", NOTE),
            (
                ".kitsu/memory/style.md",
                "+++\nkind = \"convention\"\n+++\n# Wrap errors with context\n",
            ),
        ]);
        let head = repo.git().head().expect("head").expect("some");
        let (intent, f) = notes_at(&repo, &head);
        assert_eq!(f["dedupe"], Freshness::Current);
        assert_eq!(f["style"], Freshness::Unanchored);
        assert_eq!(intent.memory["style"].title, "Wrap errors with context");

        // Unrelated change: still current.
        repo.write("README.md", "hello\n");
        let c1 = repo.commit_all("readme");
        assert_eq!(notes_at(&repo, &c1).1["dedupe"], Freshness::Current);

        // The anchored file changes: stale, and says what changed.
        repo.write("payments.py", "def charge(key): pass\n");
        let c2 = repo.commit_all("change charge");
        match &notes_at(&repo, &c2).1["dedupe"] {
            Freshness::Stale { changed, since } => {
                assert_eq!(changed, &["payments.py".to_string()]);
                assert_eq!(since, &head);
            }
            other => panic!("expected stale, got {other:?}"),
        }

        // Someone checks the note still holds and edits it: current again.
        repo.write(
            ".kitsu/memory/dedupe.md",
            &NOTE.replace("Seen in their docs.", "Seen in their docs; rechecked."),
        );
        let c3 = repo.commit_all("confirm");
        assert_eq!(notes_at(&repo, &c3).1["dedupe"], Freshness::Current);
    }

    #[test]
    fn a_note_that_isnt_committed_is_uncommitted_not_current() {
        let repo = TempRepo::new(&[("payments.py", "x\n")]);
        let git = repo.git();
        let head = git.head().expect("head").expect("some");
        repo.write(".kitsu/memory/new.md", NOTE);
        let intent = Intent::load_dir(&repo.root).expect("load");
        let f = freshness(&git, &head, intent.memory.values()).expect("freshness");
        assert_eq!(f["new"], Freshness::Uncommitted);
    }

    #[test]
    fn notes_need_a_known_kind() {
        let i = Intent::from_files(vec![
            (
                ".kitsu/memory/a.md".into(),
                b"+++\ntitle = \"x\"\n+++\n".to_vec(),
            ),
            (
                ".kitsu/memory/b.md".into(),
                b"+++\nkind = \"rumor\"\n+++\n".to_vec(),
            ),
        ]);
        assert!(i.memory.is_empty());
        assert_eq!(i.problems.len(), 2);
        assert!(
            i.problems
                .iter()
                .any(|p| p.detail.contains("needs a `kind`"))
        );
        assert!(
            i.problems
                .iter()
                .any(|p| p.detail.contains("unknown memory kind `rumor`"))
        );
    }

    #[test]
    fn personal_notes_load_and_refuse_anchors() {
        let dir = std::env::temp_dir().join(format!("kitsu-personal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("memory")).expect("dir");
        std::fs::write(
            dir.join("memory/small-commits.md"),
            "+++\nkind = \"preference\"\ntitle = \"Small commits\"\n+++\n",
        )
        .expect("w");
        std::fs::write(dir.join("memory/bad.md"), NOTE).expect("w");
        let Personal { notes, problems } = personal(&dir);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "personal/small-commits");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].1.contains("can't have anchors"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
