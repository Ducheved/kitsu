//! The architecture model, checked against the code.
//!
//! `.kitsu/architecture/<id>.md` holds one C4 element per file (person,
//! external, system, container, component), each claiming the files it
//! covers. `kitsu arch check` is an ordinary check: it fails when the model
//! and the tree disagree in ways a machine can see, which is what keeps
//! architecture docs from quietly rotting. It can't check prose; it checks
//! the claims prose rests on.
//!
//! The model is read from the tree being checked, so on an agent's snapshot
//! an agent that "fixes" a failure by editing the model is editing
//! `.kitsu/`, which review shows as a rule change.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::intent::{ElementState, Intent, Level};
use crate::scope::glob_match;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub elements: usize,
    pub covered_files: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Which levels may sit under which.
fn parent_allowed(child: Level, parent: Level) -> bool {
    matches!(
        (child, parent),
        (Level::Container, Level::System) | (Level::Component, Level::Container)
    )
}

/// `files` are the repository-relative paths present in the tree.
pub fn check(intent: &Intent, files: &[String]) -> Report {
    let els = &intent.architecture;
    let mut r = Report {
        elements: els.len(),
        ..Report::default()
    };
    let has_model = !els.is_empty() || !intent.config.architecture_cover.is_everything();
    for p in &intent.problems {
        if p.path.starts_with(".kitsu/architecture/") {
            r.errors.push(format!("{}: {}", p.path, p.detail));
        }
    }
    if !has_model {
        return r;
    }
    // A6: a check that checks nothing must say so, not pass.
    if els.is_empty() {
        r.errors.push(
            "`[architecture] cover` is set but no element loaded from .kitsu/architecture/".into(),
        );
    }

    // A1: references resolve, levels nest.
    for e in els.values() {
        match (&e.parent, e.level) {
            (None, Level::Container | Level::Component) => {
                r.errors.push(format!("`{}` is a {} with no parent", e.id, e.level.as_str()));
            }
            (Some(p), _) => match els.get(p) {
                None => r.errors.push(format!("`{}` has unknown parent `{p}`", e.id)),
                Some(pe) if !parent_allowed(e.level, pe.level) => r.errors.push(format!(
                    "`{}` is a {} under a {} (`{p}`): components go in containers, containers in systems",
                    e.id,
                    e.level.as_str(),
                    pe.level.as_str()
                )),
                Some(_) => {}
            },
            (None, _) => {}
        }
        for u in &e.uses {
            if !els.contains_key(&u.to) {
                r.errors
                    .push(format!("`{}` uses unknown element `{}`", e.id, u.to));
            } else if u.to == e.id {
                r.errors.push(format!("`{}` uses itself", e.id));
            }
        }
        for d in &e.docs {
            if !files.iter().any(|f| f == d) {
                r.errors.push(format!(
                    "`{}` points to doc `{d}`, which isn't in the tree",
                    e.id
                ));
            }
        }
    }

    // A2: what an active element claims exists.
    for e in els.values().filter(|e| e.state == ElementState::Active) {
        for g in &e.paths {
            if !files.iter().any(|f| glob_match(g, f)) {
                r.errors
                    .push(format!("`{}` claims `{g}`, which matches no file", e.id));
            }
        }
    }

    // A3: every covered file has exactly one component.
    let cover = &intent.config.architecture_cover;
    if !cover.is_everything() {
        let components: Vec<_> = els
            .values()
            .filter(|e| e.level == Level::Component && e.state != ElementState::Retired)
            .collect();
        let mut unowned = Vec::new();
        let mut shared: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for f in files.iter().filter(|f| cover.contains(f)) {
            r.covered_files += 1;
            let owners: Vec<&str> = components
                .iter()
                .filter(|c| c.paths.iter().any(|g| glob_match(g, f)))
                .map(|c| c.id.as_str())
                .collect();
            match owners.len() {
                0 => unowned.push(f.as_str()),
                1 => {}
                _ => {
                    shared.insert(f, owners.into_iter().collect());
                }
            }
        }
        if r.covered_files == 0 {
            r.errors
                .push("`[architecture] cover` matches no file".into());
        }
        for f in unowned {
            r.errors.push(format!(
                "`{f}` belongs to no component (add it to one, or add a component)"
            ));
        }
        for (f, owners) in shared {
            r.errors.push(format!(
                "`{f}` is claimed by several components: {}",
                owners.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }

    // Not errors: things worth a look.
    for e in els.values().filter(|e| e.state == ElementState::Active) {
        if e.level == Level::Component && e.paths.is_empty() {
            r.warnings
                .push(format!("`{}` claims no files, so nothing checks it", e.id));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(extra: &[(&str, &str)]) -> Intent {
        let mut files: Vec<(String, Vec<u8>)> = vec![
            (".kitsu/kitsu.toml".into(), b"[architecture]\ncover = [\"src/*.rs\"]\n".to_vec()),
            (".kitsu/architecture/sys.md".into(), b"+++\ntitle = \"Kitsu\"\nlevel = \"system\"\n+++\n".to_vec()),
            (".kitsu/architecture/cli.md".into(), b"+++\nlevel = \"container\"\nparent = \"sys\"\npaths = [\"src/**\"]\n+++\n".to_vec()),
            (".kitsu/architecture/store.md".into(), b"+++\nlevel = \"component\"\nparent = \"cli\"\npaths = [\"src/store.rs\"]\n+++\n".to_vec()),
            (".kitsu/architecture/runs.md".into(), b"+++\nlevel = \"component\"\nparent = \"cli\"\npaths = [\"src/run.rs\"]\nuses = [\"store\", { to = \"git\", why = \"worktrees\" }]\n+++\n".to_vec()),
            (".kitsu/architecture/git.md".into(), b"+++\nlevel = \"external\"\n+++\n".to_vec()),
        ];
        for (p, c) in extra {
            files.push((p.to_string(), c.as_bytes().to_vec()));
        }
        Intent::from_files(files)
    }

    fn tree(extra: &[&str]) -> Vec<String> {
        let mut t: Vec<String> = ["src/store.rs", "src/run.rs", "README.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        t.extend(extra.iter().map(|s| s.to_string()));
        t
    }

    #[test]
    fn a_consistent_model_passes() {
        let r = check(&model(&[]), &tree(&[]));
        assert!(r.ok(), "{:?}", r.errors);
        assert_eq!((r.elements, r.covered_files), (5, 2));
    }

    #[test]
    fn a_new_module_without_a_component_fails() {
        let r = check(&model(&[]), &tree(&["src/index.rs"]));
        assert_eq!(
            r.errors,
            ["`src/index.rs` belongs to no component (add it to one, or add a component)"]
        );
    }

    #[test]
    fn a_deleted_path_and_bad_references_fail() {
        let i = model(&[
            (
                ".kitsu/architecture/gone.md",
                "+++\nlevel = \"component\"\nparent = \"cli\"\npaths = [\"src/old.rs\"]\n+++\n",
            ),
            (
                ".kitsu/architecture/loose.md",
                "+++\nlevel = \"component\"\nparent = \"sys\"\nuses = [\"nobody\"]\ndocs = [\"docs/missing.md\"]\n+++\n",
            ),
            (
                ".kitsu/architecture/dup.md",
                "+++\nlevel = \"component\"\nparent = \"cli\"\npaths = [\"src/store.rs\"]\n+++\n",
            ),
        ]);
        let r = check(&i, &tree(&[]));
        let all = r.errors.join("\n");
        assert!(
            all.contains("`gone` claims `src/old.rs`, which matches no file"),
            "{all}"
        );
        assert!(
            all.contains("`loose` is a component under a system"),
            "{all}"
        );
        assert!(
            all.contains("`loose` uses unknown element `nobody`"),
            "{all}"
        );
        assert!(
            all.contains("`loose` points to doc `docs/missing.md`"),
            "{all}"
        );
        assert!(
            all.contains("`src/store.rs` is claimed by several components: dup, store"),
            "{all}"
        );
    }

    #[test]
    fn a_check_with_nothing_to_check_fails_loudly() {
        let i = Intent::from_files(vec![(
            ".kitsu/kitsu.toml".into(),
            b"[architecture]\ncover = [\"src/*.rs\"]\n".to_vec(),
        )]);
        let r = check(&i, &tree(&[]));
        assert!(
            r.errors.iter().any(|e| e.contains("no element loaded")),
            "{:?}",
            r.errors
        );
        // No model at all is not an error: the repository doesn't use this.
        assert!(check(&Intent::default(), &tree(&[])).ok());
    }

    #[test]
    fn planned_elements_may_claim_files_that_dont_exist_yet() {
        let i = model(&[(
            ".kitsu/architecture/mcp.md",
            "+++\nlevel = \"component\"\nparent = \"cli\"\nstate = \"planned\"\npaths = [\"src/mcp.rs\"]\n+++\n",
        )]);
        assert!(check(&i, &tree(&[])).ok());
    }
}
