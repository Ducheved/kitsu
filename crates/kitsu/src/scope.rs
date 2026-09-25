//! Path scopes.
//!
//! A scope is a list of repo-relative globs: `src/client/**`, `*.sql`,
//! `crates/*/Cargo.toml`. Supported syntax is deliberately small:
//! `**` (any number of path segments), `*` (anything inside one segment),
//! `?` (one character). Everything else is literal. Paths use `/`.
//!
//! Two questions get asked of scopes:
//! - does a concrete path fall inside? (exact, used against diffs)
//! - can two scopes overlap? (conservative, used to pull guarding checks into a
//!   task brief before any file has changed)

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    globs: Vec<String>,
}

impl Scope {
    pub fn new<I, S>(globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let globs = globs
            .into_iter()
            .map(|g| normalize(&g.into()))
            .filter(|g| !g.is_empty())
            .collect();
        Scope { globs }
    }

    /// An empty scope means "the whole repository".
    pub fn is_everything(&self) -> bool {
        self.globs.is_empty()
    }

    pub fn globs(&self) -> &[String] {
        &self.globs
    }

    pub fn contains(&self, path: &str) -> bool {
        self.is_everything() || self.globs.iter().any(|g| glob_match(g, path))
    }

    pub fn touches_any<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> bool {
        paths.into_iter().any(|p| self.contains(p))
    }

    /// Could some path be inside both scopes?
    ///
    /// This compares the literal directory prefixes of the globs, so it can
    /// say "yes" for scopes that never actually share a file
    /// (`src/*.rs` vs `src/*.md`). It never says "no" when they do share one.
    /// For context selection that is the right way to be wrong.
    pub fn may_overlap(&self, other: &Scope) -> bool {
        if self.is_everything() || other.is_everything() {
            return true;
        }
        self.globs.iter().any(|a| {
            other.globs.iter().any(|b| {
                let (pa, pb) = (literal_prefix(a), literal_prefix(b));
                segment_prefix(&pa, &pb) || segment_prefix(&pb, &pa)
            })
        })
    }

    /// Is every path inside `other` certainly inside `self`?
    ///
    /// The opposite lean from `may_overlap`: when it can't tell, it says
    /// no. It answers "does a check cover this rule", and an unsure yes
    /// would turn a note into something that looks enforced.
    pub fn covers(&self, other: &Scope) -> bool {
        if self.is_everything() {
            return true;
        }
        if other.is_everything() {
            return self.globs.iter().any(|h| h == "**");
        }
        other
            .globs
            .iter()
            .all(|g| self.globs.iter().any(|h| glob_covers(h, g)))
    }
}

/// Every path `inner` matches, `outer` matches too. Only two shapes are
/// understood: the same glob, and `dir/**` over anything whose leading
/// segments spell `dir` literally. Anything else is "not known to".
fn glob_covers(outer: &str, inner: &str) -> bool {
    if outer == inner || outer == "**" {
        return true;
    }
    let Some(dir) = outer.strip_suffix("/**") else {
        return false;
    };
    if dir.contains(['*', '?']) {
        return false;
    }
    let want: Vec<&str> = dir.split('/').collect();
    let segs: Vec<&str> = inner.split('/').collect();
    segs.len() > want.len()
        && want
            .iter()
            .zip(&segs)
            .all(|(w, s)| w == s && !s.contains(['*', '?']))
}

fn normalize(glob: &str) -> String {
    let g = glob.trim().trim_start_matches("./").trim_start_matches('/');
    // `dir/` means everything below it.
    if let Some(stripped) = g.strip_suffix('/') {
        format!("{stripped}/**")
    } else {
        g.to_string()
    }
}

/// The directory segments before the first wildcard.
fn literal_prefix(glob: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let segs: Vec<&str> = glob.split('/').collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if seg.contains(['*', '?']) {
            break;
        }
        // The final literal segment is a file name, not a directory, but it
        // still narrows the prefix. Keep it.
        out.push(*seg);
        if last {
            break;
        }
    }
    out
}

fn segment_prefix(short: &[&str], long: &[&str]) -> bool {
    short.len() <= long.len() && short.iter().zip(long).all(|(a, b)| a == b)
}

pub fn glob_match(glob: &str, path: &str) -> bool {
    let g: Vec<&str> = glob.split('/').collect();
    let p: Vec<&str> = path.split('/').collect();
    match_segments(&g, &p)
}

fn match_segments(g: &[&str], p: &[&str]) -> bool {
    match g.split_first() {
        None => p.is_empty(),
        Some((&"**", rest)) => {
            // `**` swallows zero or more segments.
            (0..=p.len()).any(|skip| match_segments(rest, &p[skip..]))
        }
        Some((seg, rest)) => match p.split_first() {
            Some((head, tail)) => {
                match_segment(seg.as_bytes(), head.as_bytes()) && match_segments(rest, tail)
            }
            None => false,
        },
    }
}

fn match_segment(g: &[u8], s: &[u8]) -> bool {
    match g.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|skip| match_segment(rest, &s[skip..])),
        Some((b'?', rest)) => !s.is_empty() && match_segment(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && match_segment(rest, &s[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("src/**", "src/a.rs"));
        assert!(glob_match("src/**", "src/a/b/c.rs"));
        assert!(glob_match("src/**/*.rs", "src/a.rs"));
        assert!(glob_match("src/**/*.rs", "src/x/y/a.rs"));
        assert!(!glob_match("src/**/*.rs", "src/x/y/a.md"));
        assert!(glob_match("*.sql", "schema.sql"));
        assert!(!glob_match("*.sql", "db/schema.sql"));
        assert!(glob_match("**/*.sql", "db/schema.sql"));
        assert!(glob_match("crates/*/Cargo.toml", "crates/kitsu/Cargo.toml"));
        assert!(!glob_match("crates/*/Cargo.toml", "crates/a/b/Cargo.toml"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("README.md", "README.md"));
        assert!(!glob_match("README.md", "docs/README.md"));
    }

    #[test]
    fn trailing_slash_means_subtree() {
        let s = Scope::new(["src/client/"]);
        assert!(s.contains("src/client/retry.rs"));
        assert!(!s.contains("src/server/main.rs"));
    }

    #[test]
    fn empty_scope_is_everything() {
        let s = Scope::new(Vec::<String>::new());
        assert!(s.contains("anything/at/all"));
        assert!(s.may_overlap(&Scope::new(["x/**"])));
    }

    #[test]
    fn overlap_is_conservative_but_not_silly() {
        let client = Scope::new(["src/client/**"]);
        assert!(client.may_overlap(&Scope::new(["src/client/retry.rs"])));
        assert!(client.may_overlap(&Scope::new(["src/**"])));
        assert!(client.may_overlap(&Scope::new(["**/*.rs"])));
        assert!(!client.may_overlap(&Scope::new(["src/server/**"])));
        assert!(!client.may_overlap(&Scope::new(["docs/**"])));
        // Known false positive, documented above.
        assert!(Scope::new(["src/*.rs"]).may_overlap(&Scope::new(["src/*.md"])));
    }

    #[test]
    fn covers_says_no_when_unsure() {
        let crates = Scope::new(["crates/**", "Cargo.toml"]);
        assert!(crates.covers(&Scope::new(["crates/kitsu/src/check.rs"])));
        assert!(crates.covers(&Scope::new(["crates/**", "Cargo.toml"])));
        assert!(crates.covers(&Scope::new(["crates/*/src/**"])));
        assert!(!crates.covers(&Scope::new(["crates/a.rs", "app/src/**"])));
        assert!(!crates.covers(&Scope::new(["**/*.rs"])));
        // A rule for the whole repository is covered only by a check that is.
        assert!(!crates.covers(&Scope::default()));
        assert!(Scope::default().covers(&Scope::default()));
        // Globs other than `dir/**` cover only themselves.
        assert!(!Scope::new(["**/*.rs"]).covers(&Scope::new(["src/a.rs"])));
        assert!(!Scope::new(["src/*/**"]).covers(&Scope::new(["src/a/b.rs"])));
    }
}
