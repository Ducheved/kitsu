//! The native agent's tools: schemas, and the parts that don't need the
//! host (path confinement, exact-match edits, output bounds).
//!
//! Schemas are closed (`additionalProperties: false`) and arguments are
//! parsed strictly: a model that sends something else gets the parser's
//! message back as the tool result, not a guess at what it meant.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// Every tool result is at most this long, and says so when cut.
pub const MAX_RESULT: usize = 24 * 1024;
const MAX_LINE: usize = 500;

/// What a call can change, which is what policy and crash recovery key on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads only; safe to repeat.
    None,
    /// Writes files in the worktree; the host knows the before/after.
    Worktree,
    /// Runs a process; its effects can't be known after a crash.
    Process,
}

pub fn effect(tool: &str) -> Effect {
    match tool {
        "edit_file" | "write_file" => Effect::Worktree,
        "shell" | "run_check" => Effect::Process,
        _ => Effect::None,
    }
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": props,
                "required": required,
                "additionalProperties": false,
            },
        },
    })
}

/// The tool list in the provider's (OpenAI) shape.
pub fn definitions() -> Value {
    let s = |d: &str| json!({ "type": "string", "description": d });
    let n = |d: &str| json!({ "type": "integer", "description": d });
    let b = |d: &str| json!({ "type": "boolean", "description": d });
    json!([
        tool(
            "read_file",
            "Read a file in your worktree, with line numbers. Long files come in pages: pass start_line to continue.",
            json!({ "path": s("Repository-relative path"), "start_line": n("First line, 1-based"), "end_line": n("Last line, inclusive") }),
            &["path"]
        ),
        tool(
            "list_files",
            "List files in your worktree (tracked and untracked, gitignored ones left out).",
            json!({ "path": s("Directory to list, repository-relative; default the whole repository"), "glob": s("Only paths matching this glob, e.g. src/**/*.rs"), "limit": n("At most this many, up to 1000") }),
            &[]
        ),
        tool(
            "grep",
            "Search file contents in your worktree (git grep: current contents, gitignored files left out).",
            json!({ "pattern": s("Regular expression (or plain text with fixed=true)"), "path": s("Limit to this path"), "fixed": b("Treat the pattern as plain text"), "ignore_case": b("Case-insensitive") }),
            &["pattern"]
        ),
        tool(
            "search",
            "Ranked search of the code at your base commit, by words (identifiers are split). Your own edits aren't in it; use grep for them.",
            json!({ "query": s("Words to look for"), "limit": n("At most this many results, up to 30") }),
            &["query"]
        ),
        tool(
            "rules_for",
            "The checks that guard a path and the decisions that apply to it, from the rules accept will judge by. Ask before changing a file you don't know.",
            json!({ "path": s("Repository-relative path") }),
            &["path"]
        ),
        tool(
            "git_diff",
            "What you changed since your base commit.",
            json!({ "path": s("Only this path"), "stat": b("Only the list of files with added/removed line counts") }),
            &[]
        ),
        tool(
            "edit_file",
            "Replace exact text in a file. `old` must match exactly once (or pass replace_all). If it doesn't match, read the file again: it changed or you misremembered it.",
            json!({ "path": s("Repository-relative path"), "old": s("Exact text to replace, including indentation"), "new": s("Replacement text"), "replace_all": b("Replace every occurrence") }),
            &["path", "old", "new"]
        ),
        tool(
            "write_file",
            "Create a file or replace its whole content.",
            json!({ "path": s("Repository-relative path"), "content": s("The complete new content") }),
            &["path", "content"]
        ),
        tool(
            "shell",
            "Run a shell command in your worktree (sh -c). Output is cut to the head and tail of stdout and of stderr. Processes it starts in the background are stopped when it exits. Prefer the other tools for reading, searching and editing, and run_check for checks.",
            json!({ "command": s("The command"), "timeout_secs": n("Seconds before it is killed; default 120, at most 600") }),
            &["command"]
        ),
        tool(
            "run_check",
            "Run one of the repository's checks on your worktree as it is now, and record the result as evidence.",
            json!({ "name": s("The check's name, as listed in your brief") }),
            &["name"]
        ),
        tool(
            "update_plan",
            "Record your plan. It is shown back to you in the state message, labeled as yours.",
            json!({ "entries": { "type": "array", "items": { "type": "object", "properties": { "content": { "type": "string" }, "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] } }, "required": ["content", "status"], "additionalProperties": false } } }),
            &["entries"]
        ),
        tool(
            "finish",
            "Say you are done (Kitsu then runs the required checks itself; if one fails you get the failure back and can keep working) or blocked (you can't continue without a human).",
            json!({ "outcome": { "type": "string", "enum": ["done", "blocked"] }, "summary": s("What you did, or what blocks you") }),
            &["outcome", "summary"]
        ),
    ])
}

/// Parse arguments strictly into the tool's shape.
pub fn args<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    let raw = if raw.trim().is_empty() { "{}" } else { raw };
    serde_json::from_str(raw).map_err(|e| format!("invalid arguments: {e}"))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFile {
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFiles {
    pub path: Option<String>,
    pub glob: Option<String>,
    pub limit: Option<usize>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grep {
    pub pattern: String,
    pub path: Option<String>,
    #[serde(default)]
    pub fixed: bool,
    #[serde(default)]
    pub ignore_case: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathOnly {
    pub path: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitDiff {
    pub path: Option<String>,
    #[serde(default)]
    pub stat: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    pub path: String,
    pub old: String,
    pub new: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteFile {
    pub path: String,
    pub content: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shell {
    pub command: String,
    pub timeout_secs: Option<u64>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCheck {
    pub name: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlan {
    pub entries: Vec<PlanEntry>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub outcome: String,
    pub summary: String,
}

/// A repository-relative path, resolved inside `root` or refused.
///
/// Lexical rules first (no absolute paths, no `..`, nothing under `.git`),
/// then the real one: the deepest existing ancestor is canonicalized, so a
/// symlink pointing out of the worktree is caught too.
pub fn confine(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        return Err("empty path".into());
    }
    let p = Path::new(rel);
    for c in p.components() {
        match c {
            Component::Normal(s) if s == ".git" => {
                return Err(format!("`{rel}`: .git is not yours to touch"));
            }
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(format!(
                    "`{rel}`: use a path relative to the repository, without `..`"
                ));
            }
        }
    }
    let root = root.canonicalize().map_err(|e| format!("worktree: {e}"))?;
    let full = root.join(p);
    let mut probe = full.clone();
    let mut rest = Vec::new();
    let real = loop {
        match probe.canonicalize() {
            Ok(r) => break r,
            Err(_) => {
                let Some(name) = probe.file_name().map(|n| n.to_os_string()) else {
                    return Err(format!("`{rel}`: can't resolve"));
                };
                rest.push(name);
                if !probe.pop() {
                    return Err(format!("`{rel}`: can't resolve"));
                }
            }
        }
    };
    if !real.starts_with(&root) {
        return Err(format!("`{rel}` resolves outside your worktree"));
    }
    let mut out = real;
    for n in rest.into_iter().rev() {
        out.push(n);
    }
    Ok(out)
}

/// Replace `old` with `new` in `text`. Line endings: if the file uses CRLF
/// and the arguments don't, they're converted; nothing else is fuzzy.
pub fn apply_edit(text: &str, old: &str, new: &str, all: bool) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("`old` is empty; use write_file to create a file".into());
    }
    let crlf = text.contains("\r\n") && !old.contains("\r\n");
    let (old, new) = if crlf {
        (old.replace('\n', "\r\n"), new.replace('\n', "\r\n"))
    } else {
        (old.to_string(), new.to_string())
    };
    let at: Vec<usize> = text.match_indices(&old).map(|(i, _)| i).collect();
    match at.len() {
        0 => Err(format!(
            "`old` was not found. Read the file again: it changed, or the text differs (whitespace, indentation).{}",
            nearest(text, &old)
        )),
        n if n > 1 && !all => {
            let lines: Vec<String> = at
                .iter()
                .map(|i| (text[..*i].matches('\n').count() + 1).to_string())
                .collect();
            Err(format!(
                "`old` matches {n} times (lines {}). Include more context so it matches once, or pass replace_all.",
                lines.join(", ")
            ))
        }
        n => Ok((text.replace(&old, &new), n)),
    }
}

/// Where `old` would match if whitespace at the ends of lines didn't count,
/// or where its first line is: a hint for the next call, never applied.
fn nearest(text: &str, old: &str) -> String {
    let file: Vec<&str> = text.lines().collect();
    let want: Vec<&str> = old.lines().collect();
    let at = |i: usize| {
        (0..want.len()).all(|j| file.get(i + j).is_some_and(|f| f.trim() == want[j].trim()))
    };
    let hits: Vec<usize> = (0..file.len()).filter(|&i| at(i)).collect();
    // The whitespace at the ends, made visible.
    let show = |s: &str| {
        let s = s.trim_end_matches('\r');
        let (lead, rest) = s.split_at(s.len() - s.trim_start().len());
        let (core, trail) = rest.split_at(rest.trim_end().len());
        let v = |w: &str| w.replace(' ', "·").replace('\t', "→");
        format!("{}{core}{}", v(lead), v(trail))
    };
    if let [i] = hits[..] {
        let j = (0..want.len())
            .find(|&j| file[i + j] != want[j])
            .unwrap_or(0);
        return format!(
            " It matches lines {}-{} if whitespace at the ends of lines is ignored: line {} of the file has `{}`, you sent `{}` (· is a space, → a tab).",
            i + 1,
            i + want.len(),
            i + j + 1,
            show(file[i + j]),
            show(want[j])
        );
    }
    let first = want.iter().map(|w| w.trim()).find(|w| !w.is_empty());
    let starts: Vec<usize> = (0..file.len())
        .filter(|&i| Some(file[i].trim()) == first)
        .collect();
    let list = |v: &[usize]| {
        v.iter()
            .take(5)
            .map(|i| (i + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (hits.len(), starts.is_empty()) {
        (0, true) => String::new(),
        (0, false) => format!(
            " Its first line is at line {} (whitespace aside); what follows it differs.",
            list(&starts)
        ),
        (n, _) => format!(
            " Ignoring whitespace at the ends of lines it matches {n} places, starting at lines {}.",
            list(&hits)
        ),
    }
}

/// The first line an edit touched, for the result message.
pub fn first_changed_line(before: &str, after: &str) -> usize {
    let n = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    before[..n].matches('\n').count() + 1
}

pub fn numbered(text: &str, start: usize, end: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = start.max(1);
    let end = end.unwrap_or(start + 1999).min(total).min(start + 1999);
    let mut out = String::new();
    if start > total {
        return format!("(the file has {total} lines)");
    }
    for (i, l) in lines[start - 1..end].iter().enumerate() {
        let l: String = if l.chars().count() > MAX_LINE {
            let cut: String = l.chars().take(MAX_LINE).collect();
            format!("{cut}… [line cut]")
        } else {
            l.to_string()
        };
        out.push_str(&format!("{}\t{l}\n", start + i));
        if out.len() > MAX_RESULT - 200 {
            out.push_str(&format!(
                "[kitsu: stopped at line {} of {total} to stay within the result size; continue with start_line={}]\n",
                start + i,
                start + i + 1
            ));
            return out;
        }
    }
    if end < total {
        out.push_str(&format!(
            "[kitsu: lines {start}-{end} of {total}; continue with start_line={}]\n",
            end + 1
        ));
    }
    out
}

/// Head and tail of a long output, with a marker in between.
pub fn head_tail(text: &str, head: usize, tail: usize) -> String {
    if text.len() <= head + tail {
        return text.to_string();
    }
    let h = floor_char(text, head);
    let t = ceil_char(text, text.len() - tail);
    format!(
        "{}\n[kitsu: {} bytes cut from the middle]\n{}",
        &text[..h],
        t - h,
        &text[t..]
    )
}

pub fn bounded(text: String) -> String {
    if text.len() <= MAX_RESULT {
        return text;
    }
    let cut = floor_char(&text, MAX_RESULT - 120);
    format!(
        "{}\n[kitsu: result cut at {} of {} bytes]",
        &text[..cut],
        cut,
        text.len()
    )
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_closed_and_named_uniquely() {
        let defs = definitions();
        let mut names = std::collections::BTreeSet::new();
        for d in defs.as_array().expect("list") {
            let f = &d["function"];
            assert!(names.insert(f["name"].as_str().expect("name").to_string()));
            assert_eq!(f["parameters"]["additionalProperties"], false, "{f}");
        }
        assert_eq!(names.len(), 12);
    }

    #[test]
    fn edits_are_exact_and_say_why_they_failed() {
        let t = "a\nb\na\n";
        assert!(
            apply_edit(t, "x", "y", false)
                .expect_err("should fail")
                .contains("not found")
        );
        let many = apply_edit(t, "a", "z", false).expect_err("should fail");
        assert!(many.contains("2 times (lines 1, 3)"), "{many}");
        assert_eq!(
            apply_edit(t, "a", "z", true).expect("should work"),
            ("z\nb\nz\n".into(), 2)
        );
        assert_eq!(
            apply_edit(t, "b\n", "c\n", false).expect("should work").0,
            "a\nc\na\n"
        );
        // CRLF files take LF arguments.
        assert_eq!(
            apply_edit("a\r\nb\r\n", "a\nb", "c\nd", false)
                .expect("should work")
                .0,
            "c\r\nd\r\n"
        );
        assert_eq!(first_changed_line("a\nb\nc", "a\nB\nc"), 2);
    }

    #[test]
    fn a_miss_says_where_the_nearest_text_is_and_never_applies_it() {
        let t = "def f():\n    if x:\n        return 1\n    return 2\n";
        // Wrong indentation: found when whitespace is ignored, shown, not applied.
        let e = apply_edit(t, "  if x:\n      return 1", "y", false).expect_err("a miss");
        assert!(e.contains("lines 2-3"), "{e}");
        assert!(e.contains("file has `····if x:`"), "{e}");
        // Only the first line is there.
        let e = apply_edit(t, "if x:\n    return 9", "y", false).expect_err("a miss");
        assert!(e.contains("first line") && e.contains("line 2"), "{e}");
        // Nothing like it.
        let e = apply_edit(t, "nowhere", "y", false).expect_err("a miss");
        assert!(e.contains("not found") && !e.contains("line"), "{e}");
    }

    #[test]
    fn paths_stay_in_the_worktree() {
        let root = std::env::temp_dir().join(format!("kitsu-confine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        let real = root.canonicalize().expect("canon");
        assert_eq!(
            confine(&root, "src/a.rs").expect("should work"),
            real.join("src/a.rs")
        );
        assert_eq!(
            confine(&root, "./new/dir/b.rs").expect("should work"),
            real.join("new/dir/b.rs")
        );
        for bad in ["../x", "/etc/passwd", ".git/config", "src/../../x", ""] {
            assert!(confine(&root, bad).is_err(), "{bad}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", root.join("out")).expect("symlink");
            assert!(
                confine(&root, "out/x")
                    .expect_err("should fail")
                    .contains("outside")
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn long_outputs_keep_head_and_tail() {
        let s = format!("{}{}", "h".repeat(100), "t".repeat(100));
        let ht = head_tail(&s, 10, 10);
        assert!(ht.starts_with("hhhhhhhhhh\n[kitsu: 180 bytes cut"));
        assert!(ht.ends_with("tttttttttt"));
        let n = numbered("a\nb\nc\n", 2, Some(2));
        assert_eq!(
            n,
            "2\tb\n[kitsu: lines 2-2 of 3; continue with start_line=3]\n"
        );
    }
}
