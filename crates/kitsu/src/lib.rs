//! Kitsu keeps the engineering state that coding agents lose between
//! steps: what the task is, what must stay true, what was decided and
//! rejected, and what has actually been verified. Agents are external and
//! replaceable (ACP); Kitsu owns the state and the lifecycle around them.

pub mod acp;
pub mod agent;
pub mod agents;
pub mod arch;
pub mod brief;
pub mod check;
pub mod cli;
pub mod contract;
pub mod digest;
pub mod error;
pub mod git;
pub mod hooks;
pub mod index;
pub mod integrate;
pub mod intent;
pub mod judge;
pub mod mcp;
pub mod memory;
pub mod recover;
pub mod run;
pub mod runner;
pub mod scope;
pub mod stats;
pub mod status;
pub mod store;
pub mod util;
pub mod workspace;
pub mod workspaces;

pub use error::{Error, Result};
