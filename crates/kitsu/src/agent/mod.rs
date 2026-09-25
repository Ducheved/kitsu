//! Kitsu's own agent loop.
//!
//! Through an external agent Kitsu can only put advice into someone else's
//! context, and the compaction probe showed how much of it survives (none of
//! the brief, unless it rides in the system prompt). Here Kitsu owns the
//! context: the brief is a pinned system message, the run's state is
//! re-rendered for every request, compaction is Kitsu's, and "done" is a
//! request that the checks answer.
//!
//! Two halves, one wire between them:
//! - the brain (`brain.rs`) calls the model and keeps the conversation;
//! - the host (`host.rs`) runs tools in the worktree, journals every effect
//!   before it happens, enforces the policy and verifies `finish`.
//!
//! They exchange JSON-RPC strings (`protocol.rs`) even in one process, so a
//! brain running elsewhere later is a transport change, and the host (the
//! part that touches your files) stays local either way.

pub mod anthropic;
pub mod brain;
pub mod context;
pub mod host;
pub mod login;
pub mod loops;
pub mod protocol;
pub mod provider;
pub mod responses;
pub mod tools;

use serde_json::json;
use tokio::sync::{mpsc, watch};

use crate::agents::NativeSpec;
use crate::error::Result;
use crate::run::{RunEvent, RunState};
use crate::runner::{Options, Prepared, TICK, Wake};
use crate::store::{Applied, Store};
use crate::util::content_id;
use crate::workspace::Workspace;

pub const HARNESS: &str = include_str!("harness.md");

pub async fn drive(
    ws: &Workspace,
    store: &Store,
    prep: &Prepared,
    opts: &Options,
    native: &NativeSpec,
    echo: bool,
    wake: &mut Wake,
) -> Result<RunState> {
    let id = prep.id.as_str();
    let host_of = native
        .base_url
        .split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or("");
    store.append(
        Some(id),
        "agent.config",
        &json!({
            "provider": native.provider, "host": host_of, "model": native.model,
            "api_key_env": native.api_key_env, "auth": native.auth,
            "window": native.context_window, "max_output": native.max_output,
            "turns": native.turns, "tokens": native.tokens,
            "harness_sha": content_id(HARNESS.as_bytes()),
            "tools_sha": content_id(tools::definitions().to_string().as_bytes()),
        }),
    )?;
    let provider = match provider::Provider::new(native) {
        Ok(p) => p,
        Err(e) => {
            store.apply_run_event(id, &RunEvent::StartFailed(e))?;
            return Ok(store.run(id)?.state);
        }
    };
    let judge = if opts.policy == crate::runner::Policy::Triage {
        // A broken [judge] is a broken config, not a judge that says no.
        match crate::agents::judge() {
            Ok(c) => {
                let j = crate::judge::Judge::new(c);
                store.append(Some(id), "judge.config", &j.describe())?;
                j
            }
            Err(e) => {
                store.apply_run_event(id, &RunEvent::StartFailed(format!("judge: {e}")))?;
                return Ok(store.run(id)?.state);
            }
        }
    } else {
        crate::judge::Judge::off()
    };
    let cancelled_early = matches!(
        store.apply_run_event(id, &RunEvent::Started)?,
        Applied::Unchanged
    );

    let mut host = host::Host::new(
        ws,
        store,
        id,
        &opts.agent,
        native,
        opts.policy,
        echo,
        HARNESS.to_string(),
        prep.brief.clone(),
    )?;
    host.judge = judge;
    let (tx, mut rx) = mpsc::channel::<protocol::Wire>(1);
    let link = protocol::HostLink::new(tx);
    let (cancel_tx, cancel_rx) = watch::channel(cancelled_early);
    let brain = brain::run(&link, &provider, cancel_rx);
    tokio::pin!(brain);
    let mut tick = tokio::time::interval(TICK);

    enum Ev {
        Brain(brain::End),
        Request(protocol::Wire),
        Poll,
    }
    let end = loop {
        let ev = tokio::select! {
            end = &mut brain => Ev::Brain(end),
            Some(w) = rx.recv() => Ev::Request(w),
            _ = wake.recv() => Ev::Poll,
            _ = tick.tick() => Ev::Poll,
        };
        match ev {
            Ev::Brain(end) => break end,
            Ev::Request((line, back)) => {
                // A stop that landed while the host was busy: the brain sees
                // it before it can send another model request.
                if store.run(id)?.state == RunState::Stopping {
                    let _ = cancel_tx.send(true);
                }
                let reply = host.serve(&line, wake).await;
                let _ = back.send(reply);
            }
            Ev::Poll => {
                if store.run(id)?.state == RunState::Stopping {
                    let _ = cancel_tx.send(true);
                }
            }
        }
    };
    // The host's reason wins: it's the one that ran the checks.
    let event = match (host.stop.clone(), end) {
        (Some(reason), _) => RunEvent::TurnEnded(reason),
        (None, brain::End::Stopped(reason)) => RunEvent::TurnEnded(reason),
        (None, brain::End::Failed(detail)) => RunEvent::ProtocolError(detail),
    };
    store.apply_run_event(id, &event)?;
    store.apply_run_event(id, &RunEvent::Exited("the loop ended".into()))?;
    store.set_run_pid(id, None)?;
    Ok(store.run(id)?.state)
}
