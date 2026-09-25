//! `kitsu login <provider>`: a key from the provider's own sign-in instead
//! of one pasted into an environment variable, kept in the OS keychain.
//!
//! Only sign-ins a provider documents for third-party apps are here. Today
//! that's OpenRouter's OAuth PKCE flow, which mints a key the user controls
//! (and can revoke) on their OpenRouter account. Anthropic and OpenAI don't
//! offer third-party apps a sign-in to a subscription; Kitsu drives their
//! own CLIs over ACP for that (decision `native-providers`).
//!
//! The key is never printed, logged or journaled; it goes from the token
//! exchange straight to the keychain, and from there into one header. With
//! no keychain there is no login: never a plaintext file instead.

use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Providers `kitsu login` knows, for `auth = "login:<provider>"`.
pub const PROVIDERS: &[&str] = &["openrouter"];

/// How long the browser has to come back.
const TIMEOUT: Duration = Duration::from_secs(300);

const SERVICE: &str = "kitsu";

fn entry(provider: &str) -> Result<keyring::v1::Entry, String> {
    known(provider)?;
    ready()?;
    keyring::v1::Entry::new(SERVICE, &format!("login:{provider}")).map_err(keychain_error)
}

fn known(provider: &str) -> Result<(), String> {
    if PROVIDERS.contains(&provider) {
        Ok(())
    } else {
        Err(format!(
            "no login for `{provider}` (known: {})",
            PROVIDERS.join(", ")
        ))
    }
}

/// Whether there is a keychain to keep a key in. Checked before the
/// browser opens, so a key is never minted that can't be kept.
fn ready() -> Result<(), String> {
    match keyring::v1::Entry::store_status() {
        Ok(()) => Ok(()),
        Err(e) => Err(format!(
            "the OS keychain is not available ({}); a login key is kept only there. Use api_key_env instead",
            describe(e)
        )),
    }
}

/// Keychain errors without anything they might carry of the secret.
fn describe(e: &keyring::v1::Error) -> String {
    use keyring::v1::Error as E;
    match e {
        E::PlatformFailure(d) | E::NoStorageAccess(d) => d.to_string(),
        E::NoDefaultStore => "no credential store on this platform".into(),
        E::NoEntry => "no entry".into(),
        E::BadEncoding(_) | E::BadDataFormat(..) => "the stored entry is unreadable".into(),
        _ => "keychain error".into(),
    }
}

fn keychain_error(e: keyring::v1::Error) -> String {
    format!("OS keychain: {}", describe(&e))
}

/// The key `kitsu login <provider>` stored.
pub fn stored(provider: &str) -> Result<String, String> {
    match entry(provider)?.get_password() {
        Ok(k) if !k.trim().is_empty() => Ok(k),
        Ok(_) | Err(keyring::v1::Error::NoEntry) => Err(format!(
            "not logged in to {provider}; run `kitsu login {provider}`"
        )),
        Err(e) => Err(keychain_error(e)),
    }
}

/// Forgets the stored key. The key itself stays valid until it's deleted
/// on the provider's side; this returns the page for that (OpenRouter
/// links a key's settings by its SHA-256), or `None` if there was no key.
pub fn logout(provider: &str) -> Result<Option<String>, String> {
    let entry = entry(provider)?;
    let key = match entry.get_password() {
        Ok(k) => k,
        Err(keyring::v1::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(keychain_error(e)),
    };
    entry.delete_credential().map_err(keychain_error)?;
    let hash: String = Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some(format!("https://openrouter.ai/keys/{hash}")))
}

/// Signs in through the browser and keeps the key. `say` gets what the user
/// should see (the URL to open), never the key.
pub async fn login(provider: &str, say: impl Fn(&str)) -> Result<(), String> {
    let entry = entry(provider)?;
    let verifier = random(32)?;
    let state = random(16)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("could not listen on 127.0.0.1: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("loopback listener: {e}"))?
        .port();
    let url = authorize_url(port, &state, &challenge(&verifier));
    say(&format!(
        "Opening your browser to sign in to OpenRouter. If it doesn't open, go to:\n{url}"
    ));
    open_browser(&url);
    let code = tokio::time::timeout(TIMEOUT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| format!("no answer from the browser in {}s", TIMEOUT.as_secs()))??;
    let key = exchange("https://openrouter.ai/api/v1/auth/keys", &code, &verifier).await?;
    entry.set_password(&key).map_err(keychain_error)?;
    Ok(())
}

/// OpenRouter documents localhost callbacks on any port. It has no `state`
/// parameter, so the state is part of the callback's path: only a request
/// to that exact path is taken as the answer.
fn authorize_url(port: u16, state: &str, challenge: &str) -> String {
    let callback = format!("http://localhost:{port}/callback/{state}");
    format!(
        "https://openrouter.ai/auth?callback_url={}&code_challenge={challenge}&code_challenge_method=S256&key_label=Kitsu",
        percent_encode(&callback)
    )
}

/// Answers requests on the loopback listener until one comes to
/// `/callback/<state>` with a `code` (or an `error`).
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, String> {
    let want = format!("/callback/{state}");
    loop {
        let (mut sock, _) = listener
            .accept()
            .await
            .map_err(|e| format!("loopback listener: {e}"))?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 2048];
        let head = async {
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
                match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
        };
        // Browsers open connections they may never use; one of those must
        // not hold up the one that brings the code.
        if tokio::time::timeout(Duration::from_secs(5), head)
            .await
            .is_err()
        {
            continue;
        }
        let head = String::from_utf8_lossy(&buf);
        let target = head
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split(' ').next())
            .unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != want {
            let _ = sock
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let param = |k: &str| {
            query
                .split('&')
                .filter_map(|p| p.split_once('='))
                .find(|(n, _)| *n == k)
                .map(|(_, v)| percent_decode(v))
        };
        let (page, result) = match (param("code"), param("error")) {
            (Some(code), _) if !code.is_empty() => (
                "Signed in. You can close this tab and go back to the terminal.",
                Ok(code),
            ),
            (_, e) => (
                "Sign-in did not complete. Go back to the terminal.",
                Err(format!(
                    "the sign-in came back without a code ({})",
                    e.unwrap_or_else(|| "no error given".into())
                )),
            ),
        };
        let html = format!("<!doctype html><title>Kitsu</title><p>{page}</p>");
        let _ = sock
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{html}",
                    html.len()
                )
                .as_bytes(),
            )
            .await;
        return result;
    }
}

/// The authorization code and the verifier for a key.
async fn exchange(url: &str, code: &str, verifier: &str) -> Result<String, String> {
    let http = super::provider::client()?;
    let resp = http
        .post(url)
        .header("content-type", "application/json")
        .body(
            json!({ "code": code, "code_verifier": verifier, "code_challenge_method": "S256" })
                .to_string(),
        )
        .send()
        .await
        .map_err(|e| format!("key exchange: {}", e.without_url()))?;
    let status = resp.status();
    let v: Value =
        serde_json::from_str(&resp.text().await.unwrap_or_default()).unwrap_or(Value::Null);
    match v["key"].as_str().filter(|k| !k.is_empty()) {
        Some(k) if status.is_success() => Ok(k.to_string()),
        _ => Err(format!(
            "key exchange: HTTP {}: {}",
            status.as_u16(),
            v["error"]["message"]
                .as_str()
                .unwrap_or("no key in the answer")
        )),
    }
}

fn open_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    // The URL was printed; a missing opener only means copying it.
    let _ = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `n` random bytes, base64url: a PKCE verifier (32 bytes, 43 characters)
/// or a state.
fn random(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| format!("no random bytes from the OS: {e}"))?;
    Ok(base64url(&b))
}

/// RFC 7636 S256.
fn challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let hex = || std::str::from_utf8(b.get(i + 1..i + 3)?).ok();
        match (b[i], hex().and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636() {
        // Appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        let v = random(32).expect("random");
        assert_eq!(v.len(), 43);
        assert_ne!(v, random(32).expect("random"));
    }

    #[test]
    fn the_authorize_url_carries_a_loopback_callback_with_the_state() {
        let u = authorize_url(51423, "st4te", "ch");
        assert!(u.starts_with("https://openrouter.ai/auth?callback_url=http%3A%2F%2Flocalhost%3A51423%2Fcallback%2Fst4te&"), "{u}");
        assert!(u.contains("&code_challenge=ch&code_challenge_method=S256"));
        assert_eq!(percent_decode("a%2Fb+c%zz"), "a/b c%zz");
    }

    async fn get(port: u16, target: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        s.write_all(format!("GET {target} HTTP/1.1\r\nhost: localhost\r\n\r\n").as_bytes())
            .await
            .expect("write");
        let mut out = String::new();
        s.read_to_string(&mut out).await.expect("read");
        out
    }

    #[tokio::test]
    async fn only_the_callback_with_the_right_state_answers() {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = l.local_addr().expect("addr").port();
        let client = tokio::spawn(async move {
            // A connection that never sends anything doesn't block the rest.
            let _idle = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect");
            let stray = get(port, "/favicon.ico").await;
            let forged = get(port, "/callback/other?code=evil").await;
            let real = get(port, "/callback/st4te?code=abc%2D1").await;
            (stray, forged, real)
        });
        let code = wait_for_code(&l, "st4te").await.expect("code");
        assert_eq!(code, "abc-1");
        let (stray, forged, real) = client.await.expect("client");
        assert!(stray.starts_with("HTTP/1.1 404") && forged.starts_with("HTTP/1.1 404"));
        assert!(real.contains("Signed in"));
        assert!(!real.contains("abc"), "the code isn't echoed");

        let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = l.local_addr().expect("addr").port();
        tokio::spawn(async move { get(port, "/callback/s?error=access_denied").await });
        let e = wait_for_code(&l, "s").await.expect_err("denied");
        assert!(e.contains("access_denied"), "{e}");
    }

    /// A one-request HTTP server answering `status` and `body`; returns its
    /// URL and what it was sent.
    async fn answer_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!(
            "http://127.0.0.1:{}/api/v1/auth/keys",
            l.local_addr().expect("addr").port()
        );
        let seen = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let mut got = String::new();
            while !got.contains("\"code_challenge_method\"") {
                let n = s.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(reply.as_bytes()).await.expect("write");
            got
        });
        (url, seen)
    }

    #[tokio::test]
    async fn the_code_and_verifier_are_exchanged_for_a_key() {
        let (url, seen) = answer_once("200 OK", r#"{"key":"sk-or-v1-test","user_id":"u"}"#).await;
        assert_eq!(
            exchange(&url, "c0de", "v3rifier").await.expect("key"),
            "sk-or-v1-test"
        );
        let req = seen.await.expect("request");
        assert!(req.starts_with("POST /api/v1/auth/keys"), "{req}");
        assert!(req.contains(r#""code":"c0de""#) && req.contains(r#""code_verifier":"v3rifier""#));
        assert!(req.contains(r#""code_challenge_method":"S256""#));

        let (url, _) = answer_once(
            "403 Forbidden",
            r#"{"error":{"code":403,"message":"Invalid code or code_verifier"}}"#,
        )
        .await;
        let e = exchange(&url, "c0de", "v3rifier")
            .await
            .expect_err("refused");
        assert_eq!(e, "key exchange: HTTP 403: Invalid code or code_verifier");
    }

    #[test]
    fn unknown_providers_have_no_login() {
        assert!(
            stored("anthropic")
                .expect_err("unknown")
                .contains("no login for `anthropic`")
        );
    }
}
