//! Which command starts which agent.
//!
//! Agent commands come from the user's own config, never from the
//! repository: a cloned repo must not be able to decide what binary runs
//! when you press "run". Presets cover the ACP agents known at the time of
//! writing (from the ACP registry); `~/.config/kitsu/agents.toml` overrides
//! or adds to them.
//!
//! ```toml
//! [agents.claude]
//! command = ["npx", "-y", "@agentclientprotocol/claude-agent-acp"]
//! env = { ANTHROPIC_MODEL = "..." }
//! # Sent as `_meta` on session/new. Leave it out to keep the preset's;
//! # `meta = {}` sends none.
//! meta = { systemPrompt = { excludeDynamicSections = true } }
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::workspace::config_dir;

#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Agent-specific options sent as `_meta` on `session/new`. ACP says
    /// agents must not assume anything about `_meta` keys they don't know,
    /// so this is only set for agents known to read it.
    pub meta: Option<Value>,
    /// Offer Kitsu's MCP server (`kitsu mcp`) in `session/new`. On unless
    /// an agents.toml entry says `mcp = false`.
    pub mcp: bool,
    /// Variables from Kitsu's own environment this agent may see, beyond
    /// the system basics (`BASE_ENV`). Presets name their provider's keys;
    /// agents.toml adds more with `pass_env = [...]`. Everything else in
    /// Kitsu's environment (other tokens, a host tool's session) stays out.
    pub pass_env: Vec<String>,
    pub source: &'static str,
}

/// What every agent process gets from Kitsu's environment: enough to find
/// programs, a home, a locale, a temp dir, proxies and certificates.
pub const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "SYSTEMROOT",
    "SystemRoot",
    "COMSPEC",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "PROGRAMDATA",
    // Kitsu's own tools inside the agent (`kitsu mcp`) need the same config.
    "KITSU_CONFIG_DIR",
];

/// Provider variables each preset needs to work out of the box.
fn preset_pass_env(name: &str) -> Vec<String> {
    let v: &[&str] = match name {
        "claude" => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "AWS_PROFILE",
            "AWS_REGION",
            "CLOUD_ML_REGION",
            "ANTHROPIC_VERTEX_PROJECT_ID",
        ],
        "codex" => &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"],
        "gemini" => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ],
        "opencode" => &[
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "GEMINI_API_KEY",
            "OPENCODE_CONFIG",
        ],
        "goose" => &[
            "GOOSE_PROVIDER",
            "GOOSE_MODEL",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
        ],
        "grok" => &["XAI_API_KEY"],
        "copilot" => &["GH_TOKEN", "GITHUB_TOKEN"],
        "cursor" => &["CURSOR_API_KEY"],
        // The scripted test agent reads its script from here.
        "test" => &["KITSU_TEST_SCRIPT"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// The environment an agent process starts with: the allowed part of
/// Kitsu's environment, then the agent's own `env` on top.
pub fn agent_env(
    spec: &AgentSpec,
    parent: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let allowed = |k: &str| {
        BASE_ENV.contains(&k) || k.starts_with("LC_") || spec.pass_env.iter().any(|p| p == k)
    };
    let mut out: BTreeMap<String, String> =
        parent.into_iter().filter(|(k, _)| allowed(k)).collect();
    out.extend(spec.env.clone());
    out.into_iter().collect()
}

/// `_meta` for presets that understand it.
///
/// Claude Code's system prompt normally carries the working directory and
/// git status. Every Kitsu run has its own worktree, so that prompt differs
/// on every run and the provider's prompt cache misses on the whole system
/// prompt and tool list, every time. With the dynamic sections moved into
/// the first user message the prefix is identical across runs. The brief
/// states the worktree path itself, so the agent doesn't lose it.
fn preset_meta(name: &str) -> Option<Value> {
    match name {
        "claude" => Some(json!({ "systemPrompt": { "excludeDynamicSections": true } })),
        _ => None,
    }
}

const PRESETS: &[(&str, &[&str])] = &[
    (
        "claude",
        &["npx", "-y", "@agentclientprotocol/claude-agent-acp"],
    ),
    ("codex", &["npx", "-y", "@agentclientprotocol/codex-acp"]),
    ("gemini", &["gemini", "--acp"]),
    ("opencode", &["opencode", "acp"]),
    ("goose", &["goose", "acp"]),
    ("grok", &["grok", "agent", "stdio"]),
    ("copilot", &["copilot", "--acp"]),
    ("cursor", &["cursor-agent", "acp"]),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    agents: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    command: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    meta: Option<toml::Table>,
    mcp: Option<bool>,
    #[serde(default)]
    pass_env: Vec<String>,
}

pub fn all() -> Result<Vec<AgentSpec>> {
    let mut map: BTreeMap<String, AgentSpec> = BTreeMap::new();
    for (name, cmd) in PRESETS {
        map.insert(
            name.to_string(),
            AgentSpec {
                name: name.to_string(),
                command: cmd.iter().map(|s| s.to_string()).collect(),
                env: BTreeMap::new(),
                meta: preset_meta(name),
                mcp: true,
                pass_env: preset_pass_env(name),
                source: "preset",
            },
        );
    }
    if let Some(test) = test_agent_path() {
        map.insert(
            "test".into(),
            AgentSpec {
                name: "test".into(),
                command: vec![test],
                env: BTreeMap::new(),
                meta: None,
                mcp: true,
                pass_env: preset_pass_env("test"),
                source: "bundled",
            },
        );
    }
    let path = config_dir().join("agents.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => apply_config(&mut map, &path, &text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io(path.display().to_string(), e)),
    }
    Ok(map.into_values().collect())
}

fn apply_config(
    map: &mut BTreeMap<String, AgentSpec>,
    path: &std::path::Path,
    text: &str,
) -> Result<()> {
    let bad = |detail: String| Error::Parse {
        path: path.to_path_buf(),
        detail,
    };
    let file: File = toml::from_str(text).map_err(|e| bad(e.message().to_string()))?;
    for (name, e) in file.agents {
        if e.command.is_empty() {
            return Err(bad(format!("agent `{name}` has an empty command")));
        }
        let pass_env: Vec<String> = preset_pass_env(&name)
            .into_iter()
            .chain(e.pass_env)
            .collect();
        let meta = match e.meta {
            None => preset_meta(&name),
            Some(t) if t.is_empty() => None,
            Some(t) => Some(
                serde_json::to_value(t)
                    .map_err(|err| bad(format!("agent `{name}` meta: {err}")))?,
            ),
        };
        map.insert(
            name.clone(),
            AgentSpec {
                name,
                command: e.command,
                env: e.env,
                meta,
                mcp: e.mcp.unwrap_or(true),
                pass_env,
                source: "agents.toml",
            },
        );
    }
    Ok(())
}

pub fn resolve(name: &str) -> Result<AgentSpec> {
    all()?.into_iter().find(|a| a.name == name).ok_or_else(|| {
        Error::NotFound(format!(
            "agent `{name}` (add it to {})",
            config_dir().join("agents.toml").display()
        ))
    })
}

/// The bundled deterministic agent sits next to the running binary.
fn test_agent_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "kitsu-test-agent.exe"
    } else {
        "kitsu-test-agent"
    };
    // Test binaries live one level down in target/<profile>/deps.
    [dir.join(name), dir.parent()?.join(name)]
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_config(text: &str) -> BTreeMap<String, AgentSpec> {
        let mut map: BTreeMap<String, AgentSpec> = PRESETS
            .iter()
            .map(|(n, c)| {
                (
                    n.to_string(),
                    AgentSpec {
                        name: n.to_string(),
                        command: c.iter().map(|s| s.to_string()).collect(),
                        env: BTreeMap::new(),
                        meta: preset_meta(n),
                        mcp: true,
                        pass_env: preset_pass_env(n),
                        source: "preset",
                    },
                )
            })
            .collect();
        apply_config(&mut map, std::path::Path::new("agents.toml"), text).expect("config");
        map
    }

    #[test]
    fn claude_gets_a_cacheable_system_prompt_and_overrides_keep_it() {
        let presets = with_config("");
        assert_eq!(
            presets["claude"].meta.as_ref().expect("meta")["systemPrompt"]["excludeDynamicSections"],
            true
        );
        assert!(
            presets["codex"].meta.is_none(),
            "unknown _meta is not sent to agents that don't read it"
        );

        let over = with_config(
            "[agents.claude]\ncommand = [\"claude-acp\"]\nenv = { ANTHROPIC_MODEL = \"x\" }\n",
        );
        assert!(
            over["claude"].meta.is_some(),
            "changing the command doesn't silently drop the preset's meta"
        );

        let off = with_config("[agents.claude]\ncommand = [\"claude-acp\"]\nmeta = {}\n");
        assert!(off["claude"].meta.is_none(), "meta = {{}} turns it off");

        let env_of = |spec: &AgentSpec| {
            agent_env(
                spec,
                [
                    ("PATH", "/bin"),
                    ("ANTHROPIC_API_KEY", "k"),
                    ("GITHUB_TOKEN", "t"),
                    ("HOST_SESSION_TOKEN", "s"),
                    ("LC_ALL", "C"),
                ]
                .map(|(k, v)| (k.to_string(), v.to_string())),
            )
            .into_iter()
            .map(|(k, _)| k)
            .collect::<Vec<_>>()
        };
        assert_eq!(
            env_of(&presets["claude"]),
            ["ANTHROPIC_API_KEY", "LC_ALL", "PATH"],
            "no other tokens, no host session"
        );
        assert_eq!(
            env_of(&presets["codex"]),
            ["LC_ALL", "PATH"],
            "codex doesn't get the Anthropic key"
        );
        let widened =
            with_config("[agents.claude]\ncommand = [\"x\"]\npass_env = [\"GITHUB_TOKEN\"]\n");
        assert!(env_of(&widened["claude"]).contains(&"GITHUB_TOKEN".to_string()));

        let custom =
            with_config("[agents.mine]\ncommand = [\"x\"]\nmeta = { mode = \"fast\", n = 2 }\n");
        assert_eq!(custom["mine"].meta, Some(json!({ "mode": "fast", "n": 2 })));
    }
}
