//! Sets `KITSU_VERSION`, the version `kitsu --version` prints.
//!
//! A release prints the Cargo version. A CI build that isn't a release sets
//! `KITSU_BUILD_VERSION` (computed by `.github/scripts/version.sh`) to
//! something like `0.1.1-dev.14+g1a2b3c4`, so the binary says which commit
//! it came from. Unset, empty or not SemVer 2.0.0 falls back to the Cargo
//! version: this script never fails a build.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=KITSU_BUILD_VERSION");
    let cargo = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = match env::var("KITSU_BUILD_VERSION") {
        Ok(v) if v.trim().is_empty() => cargo,
        Ok(v) if is_semver(v.trim()) => v.trim().to_string(),
        Ok(v) => {
            println!(
                "cargo::warning=ignoring KITSU_BUILD_VERSION={v:?}: not a SemVer 2.0.0 version, using {cargo}"
            );
            cargo
        }
        Err(_) => cargo,
    };
    println!("cargo::rustc-env=KITSU_VERSION={version}");
}

/// `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` as semver.org 2.0.0 defines it.
fn is_semver(v: &str) -> bool {
    let (rest, build) = match v.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (v, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let numeric = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
    };
    let ident =
        |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let core_ok = {
        let parts: Vec<&str> = core.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| numeric(p))
    };
    // A numeric pre-release identifier has no leading zeros; build
    // identifiers may.
    let pre_ok = pre.is_none_or(|p| {
        p.split('.')
            .all(|i| ident(i) && (!i.bytes().all(|b| b.is_ascii_digit()) || numeric(i)))
    });
    let build_ok = build.is_none_or(|b| b.split('.').all(ident));
    core_ok && pre_ok && build_ok
}
