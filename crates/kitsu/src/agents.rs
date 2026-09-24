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
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::workspace::config_dir;

#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub source: &'static str,
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
                source: "bundled",
            },
        );
    }
    let path = config_dir().join("agents.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let file: File = toml::from_str(&text).map_err(|e| Error::Parse {
                path: path.clone(),
                detail: e.message().to_string(),
            })?;
            for (name, e) in file.agents {
                if e.command.is_empty() {
                    return Err(Error::Parse {
                        path: path.clone(),
                        detail: format!("agent `{name}` has an empty command"),
                    });
                }
                map.insert(
                    name.clone(),
                    AgentSpec {
                        name,
                        command: e.command,
                        env: e.env,
                        source: "agents.toml",
                    },
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io(path.display().to_string(), e)),
    }
    Ok(map.into_values().collect())
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
