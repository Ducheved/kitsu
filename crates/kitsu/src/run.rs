//! The run lifecycle as a pure reducer.
//!
//! A run is one attempt by one agent at one task, in its own worktree. This
//! module only decides which transitions are legal. It does no I/O, so every
//! (state, event) pair can be tested exhaustively.
//!
//! What a run ending means is deliberately narrow: `Finished` says the agent
//! returned from its turn. It does not say the work is correct. Correctness
//! comes from evidence, and acceptance comes from a human.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Intent recorded. Worktree and agent process may or may not exist yet.
    Starting,
    Running,
    /// Cancel was requested and forwarded; waiting for the agent to wind down.
    Stopping,
    /// The agent answered its prompt turn (any stop reason, including
    /// `cancelled`).
    Finished,
    /// Spawn failed, the process died mid-turn, or it broke the protocol.
    Failed,
    /// The process that owned this run disappeared. What the agent did after
    /// that point is unknown; the worktree is what we have.
    Interrupted,
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Finished | RunState::Failed | RunState::Interrupted
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Starting => "starting",
            RunState::Running => "running",
            RunState::Stopping => "stopping",
            RunState::Finished => "finished",
            RunState::Failed => "failed",
            RunState::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<RunState> {
        Some(match s {
            "starting" => RunState::Starting,
            "running" => RunState::Running,
            "stopping" => RunState::Stopping,
            "finished" => RunState::Finished,
            "failed" => RunState::Failed,
            "interrupted" => RunState::Interrupted,
            _ => return None,
        })
    }

    pub const ALL: [RunState; 6] = [
        RunState::Starting,
        RunState::Running,
        RunState::Stopping,
        RunState::Finished,
        RunState::Failed,
        RunState::Interrupted,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// Agent process is up and the prompt was sent.
    Started,
    StartFailed(String),
    /// `session/prompt` returned with this stop reason.
    TurnEnded(String),
    CancelRequested,
    /// The agent process exited. Expected after a turn ends; a failure before.
    Exited(String),
    ProtocolError(String),
    /// Recovery found the owning process gone.
    OwnerLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// State changes. `detail` goes into the run row.
    Move {
        to: RunState,
        stop_reason: Option<String>,
        detail: Option<String>,
    },
    /// Legal but changes nothing (a repeated cancel, the normal exit after a
    /// finished turn). Recorded as an event, state untouched.
    Same,
    /// A second completion for a run that already finished. Not an error for
    /// the caller, but it is recorded, because an agent that answers twice
    /// is telling us something.
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Illegal {
    pub from: RunState,
    pub event: RunEvent,
}

impl std::fmt::Display for Illegal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "run is {} and cannot take {:?}",
            self.from.as_str(),
            self.event
        )
    }
}

pub fn reduce(from: RunState, event: &RunEvent) -> Result<Outcome, Illegal> {
    use RunEvent as E;
    use RunState as S;
    let illegal = || {
        Err(Illegal {
            from,
            event: event.clone(),
        })
    };
    let to = |to: RunState| {
        Ok(Outcome::Move {
            to,
            stop_reason: None,
            detail: None,
        })
    };
    let fail = |d: &String| {
        Ok(Outcome::Move {
            to: S::Failed,
            stop_reason: None,
            detail: Some(d.clone()),
        })
    };
    match (from, event) {
        (S::Starting, E::Started) => to(S::Running),
        // Cancel raced the start. The starter sees Stopping and cancels the
        // agent as soon as the session exists.
        (S::Stopping, E::Started) => Ok(Outcome::Same),
        (S::Starting | S::Stopping, E::StartFailed(d)) => fail(d),
        (S::Starting, E::CancelRequested) => to(S::Stopping),

        (S::Running | S::Stopping, E::TurnEnded(reason)) => Ok(Outcome::Move {
            to: S::Finished,
            stop_reason: Some(reason.clone()),
            detail: None,
        }),
        (S::Running, E::CancelRequested) => to(S::Stopping),
        (S::Stopping, E::CancelRequested) => Ok(Outcome::Same),
        (S::Running | S::Stopping, E::Exited(d)) => fail(d),
        (S::Starting | S::Running | S::Stopping, E::ProtocolError(d)) => fail(d),
        (S::Starting | S::Running | S::Stopping, E::OwnerLost) => to(S::Interrupted),

        (S::Finished, E::TurnEnded(_)) => Ok(Outcome::Duplicate),
        // Processes exit after their turn; that is the normal ending.
        (S::Finished | S::Failed | S::Interrupted, E::Exited(_)) => Ok(Outcome::Same),
        // Late noise after the run is over is recorded, not acted on.
        (S::Finished | S::Failed | S::Interrupted, E::ProtocolError(_)) => Ok(Outcome::Same),
        (S::Finished | S::Failed | S::Interrupted, E::OwnerLost) => Ok(Outcome::Same),

        // Everything else is a bug in the caller: starting twice, cancelling
        // something that already ended, completing a run that never started.
        _ => illegal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> Vec<RunEvent> {
        vec![
            RunEvent::Started,
            RunEvent::StartFailed("x".into()),
            RunEvent::TurnEnded("end_turn".into()),
            RunEvent::CancelRequested,
            RunEvent::Exited("code 0".into()),
            RunEvent::ProtocolError("bad json".into()),
            RunEvent::OwnerLost,
        ]
    }

    /// The full table. If someone changes a transition, this test forces
    /// them to change the expectation next to it.
    #[test]
    fn transition_table() {
        use RunState::*;
        let expect = |s: RunState, e: &RunEvent| -> &'static str {
            match (s, e) {
                (Starting, RunEvent::Started) => "running",
                (Starting, RunEvent::StartFailed(_)) => "failed",
                (Starting, RunEvent::TurnEnded(_)) => "illegal",
                (Starting, RunEvent::CancelRequested) => "stopping",
                (Starting, RunEvent::Exited(_)) => "illegal",
                (Starting, RunEvent::ProtocolError(_)) => "failed",
                (Starting, RunEvent::OwnerLost) => "interrupted",

                (Running, RunEvent::Started) => "illegal",
                (Running, RunEvent::StartFailed(_)) => "illegal",
                (Running, RunEvent::TurnEnded(_)) => "finished",
                (Running, RunEvent::CancelRequested) => "stopping",
                (Running, RunEvent::Exited(_)) => "failed",
                (Running, RunEvent::ProtocolError(_)) => "failed",
                (Running, RunEvent::OwnerLost) => "interrupted",

                (Stopping, RunEvent::Started) => "same",
                (Stopping, RunEvent::StartFailed(_)) => "failed",
                (Stopping, RunEvent::TurnEnded(_)) => "finished",
                (Stopping, RunEvent::CancelRequested) => "same",
                (Stopping, RunEvent::Exited(_)) => "failed",
                (Stopping, RunEvent::ProtocolError(_)) => "failed",
                (Stopping, RunEvent::OwnerLost) => "interrupted",

                (Finished, RunEvent::TurnEnded(_)) => "duplicate",
                (Finished | Failed | Interrupted, RunEvent::Exited(_)) => "same",
                (Finished | Failed | Interrupted, RunEvent::ProtocolError(_)) => "same",
                (Finished | Failed | Interrupted, RunEvent::OwnerLost) => "same",
                (Finished | Failed | Interrupted, _) => "illegal",
            }
        };
        for s in RunState::ALL {
            for e in events() {
                let got = match reduce(s, &e) {
                    Ok(Outcome::Move { to, .. }) => to.as_str(),
                    Ok(Outcome::Same) => "same",
                    Ok(Outcome::Duplicate) => "duplicate",
                    Err(_) => "illegal",
                };
                assert_eq!(got, expect(s, &e), "{s:?} + {e:?}");
            }
        }
    }

    #[test]
    fn terminal_states_never_leave() {
        for s in RunState::ALL.into_iter().filter(|s| s.is_terminal()) {
            for e in events() {
                if let Ok(Outcome::Move { to, .. }) = reduce(s, &e) {
                    panic!("{s:?} moved to {to:?} on {e:?}");
                }
            }
        }
    }

    #[test]
    fn completion_after_cancel_is_recorded_as_what_happened() {
        // The user asked to stop; the agent had already finished. The run is
        // Finished with the agent's real stop reason, not "cancelled".
        let s = match reduce(RunState::Running, &RunEvent::CancelRequested) {
            Ok(Outcome::Move { to, .. }) => to,
            other => panic!("{other:?}"),
        };
        match reduce(s, &RunEvent::TurnEnded("end_turn".into())) {
            Ok(Outcome::Move {
                to: RunState::Finished,
                stop_reason: Some(r),
                ..
            }) => assert_eq!(r, "end_turn"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn states_round_trip() {
        for s in RunState::ALL {
            assert_eq!(RunState::parse(s.as_str()), Some(s));
        }
    }
}
