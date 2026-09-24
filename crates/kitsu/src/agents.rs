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
//!
//! # For every agent and everything it starts (searches, builds, tests).
//! [limits]
//! nice = 10   # 0 to turn off
//! cpus = 2    # 0: no cap; unset: all but one when there are 3 or more
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
    #[serde(default)]
    limits: LimitsFront,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LimitsFront {
    nice: Option<u8>,
    cpus: Option<usize>,
}

/// How hard an agent, and everything it starts, may lean on the machine.
///
/// Agents run their own ripgrep, builds and tests, and several runs can be
/// going at once. Measured with three agents' worth of ripgrep bursts over
/// a 1M-line tree on 4 cores: unlimited, a 5 ms foreground frame finished
/// up to 24-28 ms late at p99; at nice 10, 9-14 ms with the same
/// throughput; at nice 10 with one core left out, 6-9 ms for 10% more wall
/// time. Agents' own ripgrep config can't do this: Claude Code's native
/// build runs its ripgrep with `--no-config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Added to Kitsu's own niceness (0-19). The agent still gets idle
    /// CPU; it yields when something else needs it.
    pub nice: u8,
    /// How many CPUs the agent may run on. `None`: all but one when there
    /// are at least three. `Some(0)`: all of them.
    pub cpus: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            nice: 10,
            cpus: None,
        }
    }
}

/// `[limits]` from agents.toml, or the defaults.
pub fn limits() -> Result<Limits> {
    let path = config_dir().join("agents.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_limits(&path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Limits::default()),
        Err(e) => Err(Error::io(path.display().to_string(), e)),
    }
}

fn parse_limits(path: &std::path::Path, text: &str) -> Result<Limits> {
    let bad = |detail: String| Error::Parse {
        path: path.to_path_buf(),
        detail,
    };
    let file: File = toml::from_str(text).map_err(|e| bad(e.message().to_string()))?;
    let d = Limits::default();
    let nice = file.limits.nice.unwrap_or(d.nice);
    if nice > 19 {
        return Err(bad(format!("limits.nice is {nice}; it goes from 0 to 19")));
    }
    Ok(Limits {
        nice,
        cpus: file.limits.cpus,
    })
}

/// The command line that starts `command` under `limits`, and a line for
/// each limit saying what was applied or why it wasn't.
///
/// Wrappers that exec (`nice`, `taskset`), so the agent keeps the pid and
/// process group Kitsu started, and every child inherits the limits.
/// `allowed` are the CPUs Kitsu itself may run on; `have` says whether a
/// program is on the PATH.
pub fn limited(
    command: &[String],
    limits: Limits,
    allowed: &[usize],
    have: impl Fn(&str) -> bool,
) -> (Vec<String>, Vec<String>) {
    let mut prefix: Vec<String> = Vec::new();
    let mut applied = Vec::new();
    if limits.nice > 0 {
        if cfg!(unix) && have("nice") {
            prefix.extend(["nice".into(), "-n".into(), limits.nice.to_string()]);
            applied.push(format!("nice {}", limits.nice));
        } else {
            applied.push("nice: not applied (no `nice` here)".into());
        }
    }
    let n = allowed.len();
    let want = match limits.cpus {
        Some(0) => n,
        Some(k) => k.min(n),
        None if n >= 3 => n - 1,
        None => n,
    };
    if want < n {
        if cfg!(target_os = "linux") && have("taskset") {
            let list: Vec<String> = allowed[..want].iter().map(|c| c.to_string()).collect();
            prefix.extend(["taskset".into(), "-c".into(), list.join(",")]);
            applied.push(format!("cpus {want} of {n}"));
        } else {
            applied.push(format!(
                "cpus: not capped to {want} of {n} (no `taskset` here)"
            ));
        }
    }
    prefix.extend(command.iter().cloned());
    (prefix, applied)
}

/// The CPUs this process may run on: Linux's `Cpus_allowed_list`, else
/// `0..available_parallelism`.
pub fn allowed_cpus() -> Vec<usize> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(list) = status
            .lines()
            .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
        && let Some(cpus) = parse_cpu_list(list.trim())
    {
        return cpus;
    }
    let n = std::thread::available_parallelism().map_or(1, |n| n.get());
    (0..n).collect()
}

/// `0-3,8,10-11` → [0, 1, 2, 3, 8, 10, 11]. `None` if it doesn't parse.
fn parse_cpu_list(s: &str) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in s.split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => out.extend(a.parse::<usize>().ok()?..=b.parse::<usize>().ok()?),
            None => out.push(part.parse().ok()?),
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Whether `program` is an executable file on the PATH.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
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

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn limits_wrap_the_command_with_programs_that_exec() {
        let cmd = argv(&["claude-agent-acp", "--x"]);
        let all = |_: &str| true;
        let (line, applied) = limited(&cmd, Limits::default(), &[0, 1, 2, 3], all);
        if cfg!(target_os = "linux") {
            assert_eq!(
                line,
                argv(&[
                    "nice",
                    "-n",
                    "10",
                    "taskset",
                    "-c",
                    "0,1,2",
                    "claude-agent-acp",
                    "--x"
                ])
            );
            assert_eq!(applied, ["nice 10", "cpus 3 of 4"]);
        }
        // The CPUs are the ones this process may use, not 0..n.
        let (line, _) = limited(
            &cmd,
            Limits {
                nice: 0,
                cpus: Some(2),
            },
            &[4, 5, 6, 7],
            all,
        );
        if cfg!(target_os = "linux") {
            assert_eq!(
                line,
                argv(&["taskset", "-c", "4,5", "claude-agent-acp", "--x"])
            );
        }
        // Two CPUs: nothing to leave out by default; `cpus = 0` never caps.
        let off = Limits {
            nice: 0,
            cpus: None,
        };
        assert_eq!(limited(&cmd, off, &[0, 1], all).0, cmd);
        let off = Limits {
            nice: 0,
            cpus: Some(0),
        };
        assert_eq!(limited(&cmd, off, &[0, 1, 2, 3], all).0, cmd);
    }

    #[test]
    fn a_missing_wrapper_is_reported_not_skipped_silently() {
        let cmd = argv(&["agent"]);
        let (line, applied) = limited(&cmd, Limits::default(), &[0, 1, 2, 3], |_| false);
        assert_eq!(line, cmd);
        assert_eq!(
            applied,
            [
                "nice: not applied (no `nice` here)",
                "cpus: not capped to 3 of 4 (no `taskset` here)"
            ]
        );
    }

    #[test]
    fn cpu_lists_and_limit_config_parse() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(parse_cpu_list("x"), None);
        let p = std::path::Path::new("agents.toml");
        assert_eq!(parse_limits(p, "").expect("empty"), Limits::default());
        assert_eq!(
            parse_limits(p, "[limits]\nnice = 0\ncpus = 2\n").expect("set"),
            Limits {
                nice: 0,
                cpus: Some(2)
            }
        );
        assert!(parse_limits(p, "[limits]\nnice = 25\n").is_err());
        assert!(parse_limits(p, "[limits]\nthreads = 2\n").is_err());
    }
}
