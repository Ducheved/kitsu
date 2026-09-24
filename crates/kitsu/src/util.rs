use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Short content identity: the first 16 hex chars of SHA-256. Used for
/// provenance and artifact names, not for security.
pub fn content_id(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex(&digest[..8])
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Wall-clock milliseconds. Only ever used for display and for "since you
/// left"; ordering always comes from sequence numbers.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Short, human-typable random id like `r4f7kq`. Uniqueness is enforced by
/// the database primary key; callers retry on collision.
pub fn short_id(prefix: char) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    let mut n = h.finish();
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut s = String::with_capacity(7);
    s.push(prefix);
    for _ in 0..6 {
        s.push(ALPHABET[(n % 32) as usize] as char);
        n /= 32;
    }
    s
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

pub fn ago(then_ms: i64, now: i64) -> String {
    let s = ((now - then_ms).max(0) / 1000) as u64;
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Make payment retries idempotent!"),
            "make-payment-retries-idempotent"
        );
        assert_eq!(slugify("  --  "), "untitled");
        assert_eq!(slugify("Ünïcode & stuff"), "n-code-stuff");
    }

    #[test]
    fn ids_are_short_and_distinct() {
        let a = short_id('r');
        let b = short_id('r');
        assert_eq!(a.len(), 7);
        assert_ne!(a, b);
    }
}
