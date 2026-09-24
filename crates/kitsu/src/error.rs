use std::fmt;
use std::path::PathBuf;

/// Errors keep their category across boundaries. The UI and the CLI react
/// differently to a malformed file, a refused transition, a moved branch and
/// a crashed subprocess, so none of them collapse into a string.
#[derive(Debug)]
pub enum Error {
    /// The request itself is wrong (bad id, bad argument, illegal transition).
    Invalid(String),
    /// A `.kitsu/` file could not be parsed. Always points at the file.
    Parse {
        path: PathBuf,
        detail: String,
    },
    NotFound(String),
    /// Somebody else moved first: CAS failure, stale base, state changed
    /// underneath us. Safe to re-read and retry the whole operation.
    Conflict(String),
    /// Policy said no (untrusted repo, protected path without approval).
    Denied(String),
    /// A git invocation failed. Carries the arguments and stderr verbatim.
    Git {
        args: String,
        code: Option<i32>,
        stderr: String,
    },
    /// The agent broke the protocol contract.
    Protocol(String),
    Io {
        context: String,
        source: std::io::Error,
    },
    Db(rusqlite::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    /// Short machine-readable category, used by the UI and `--json` output.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid",
            Error::Parse { .. } => "parse",
            Error::NotFound(_) => "not_found",
            Error::Conflict(_) => "conflict",
            Error::Denied(_) => "denied",
            Error::Git { .. } => "git",
            Error::Protocol(_) => "protocol",
            Error::Io { .. } => "io",
            Error::Db(_) => "db",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "{m}"),
            Error::Parse { path, detail } => write!(f, "{}: {detail}", path.display()),
            Error::NotFound(m) => write!(f, "not found: {m}"),
            Error::Conflict(m) => write!(f, "conflict: {m}"),
            Error::Denied(m) => write!(f, "denied: {m}"),
            Error::Git { args, code, stderr } => {
                let code = code.map_or_else(|| "signal".to_string(), |c| c.to_string());
                write!(f, "git {args} failed ({code}): {}", stderr.trim())
            }
            Error::Protocol(m) => write!(f, "agent protocol violation: {m}"),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Db(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}
