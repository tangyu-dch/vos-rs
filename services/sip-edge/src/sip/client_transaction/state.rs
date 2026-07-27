#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientTransactionKind {
    Invite,
    NonInvite,
}

impl ClientTransactionKind {
    pub(super) fn from_method(method: &str) -> Self {
        if method.eq_ignore_ascii_case("INVITE") {
            Self::Invite
        } else {
            Self::NonInvite
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum ClientTransactionState {
    Calling,
    Trying,
    Proceeding,
    Completed,
    Terminated,
}

impl ClientTransactionState {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Calling,
            1 => Self::Trying,
            2 => Self::Proceeding,
            3 => Self::Completed,
            _ => Self::Terminated,
        }
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Terminated)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponseAction {
    Ignore,
    StopRetransmissions,
    Complete,
}

#[derive(Debug)]
#[cfg(test)]
pub(super) struct ClientTransactionMachine {
    kind: ClientTransactionKind,
    state: ClientTransactionState,
}

#[derive(Debug)]
pub(super) struct ClientTransactionControl {
    kind: ClientTransactionKind,
    state: AtomicU8,
    changed: Notify,
}

impl ClientTransactionControl {
    pub(super) fn new(method: &str) -> Self {
        let kind = ClientTransactionKind::from_method(method);
        let state = match kind {
            ClientTransactionKind::Invite => ClientTransactionState::Calling,
            ClientTransactionKind::NonInvite => ClientTransactionState::Trying,
        };
        Self {
            kind,
            state: AtomicU8::new(state as u8),
            changed: Notify::new(),
        }
    }

    pub(super) fn on_response(&self, status_code: u16) -> ResponseAction {
        if !(100..=699).contains(&status_code) || self.state().is_terminal() {
            return ResponseAction::Ignore;
        }

        // UDP peers commonly repeat provisional responses. Once an INVITE is
        // already Proceeding, publishing another notification would make the
        // runner repeatedly wake itself while consulting the response ledger.
        if status_code < 200 && self.state() == ClientTransactionState::Proceeding {
            return ResponseAction::Ignore;
        }

        let (next_state, action) = if status_code < 200 {
            let action = match self.kind {
                ClientTransactionKind::Invite => ResponseAction::StopRetransmissions,
                ClientTransactionKind::NonInvite => ResponseAction::Ignore,
            };
            (ClientTransactionState::Proceeding, action)
        } else {
            (ClientTransactionState::Completed, ResponseAction::Complete)
        };
        self.state.store(next_state as u8, Ordering::Release);
        self.changed.notify_one();
        action
    }

    pub(super) fn cancel(&self) {
        self.state
            .store(ClientTransactionState::Terminated as u8, Ordering::Release);
        self.changed.notify_one();
    }

    pub(super) fn should_retransmit(&self) -> bool {
        match self.kind {
            ClientTransactionKind::Invite => self.state() == ClientTransactionState::Calling,
            ClientTransactionKind::NonInvite => matches!(
                self.state(),
                ClientTransactionState::Trying | ClientTransactionState::Proceeding
            ),
        }
    }

    /// Returns whether the RFC 3261 transaction timeout is still applicable.
    ///
    /// Timer B is stopped when an INVITE receives a provisional response and
    /// enters Proceeding. The transaction then remains alive for the final
    /// response instead of being discarded after 32 seconds of ringing.
    pub(super) fn should_timeout(&self) -> bool {
        match self.kind {
            ClientTransactionKind::Invite => self.state() == ClientTransactionState::Calling,
            ClientTransactionKind::NonInvite => matches!(
                self.state(),
                ClientTransactionState::Trying | ClientTransactionState::Proceeding
            ),
        }
    }

    pub(super) fn is_terminal(&self) -> bool {
        self.state().is_terminal()
    }

    pub(super) fn state(&self) -> ClientTransactionState {
        ClientTransactionState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }
}

#[cfg(test)]
impl ClientTransactionMachine {
    pub(super) fn new(method: &str) -> Self {
        let kind = ClientTransactionKind::from_method(method);
        let state = match kind {
            ClientTransactionKind::Invite => ClientTransactionState::Calling,
            ClientTransactionKind::NonInvite => ClientTransactionState::Trying,
        };
        Self { kind, state }
    }

    pub(super) fn on_response(&mut self, status_code: u16) -> ResponseAction {
        if !(100..=699).contains(&status_code) {
            return ResponseAction::Ignore;
        }

        if status_code < 200 {
            self.state = ClientTransactionState::Proceeding;
            return match self.kind {
                ClientTransactionKind::Invite => ResponseAction::StopRetransmissions,
                ClientTransactionKind::NonInvite => ResponseAction::Ignore,
            };
        }

        self.state = ClientTransactionState::Completed;
        ResponseAction::Complete
    }

    pub(super) fn should_retransmit(&self) -> bool {
        match self.kind {
            ClientTransactionKind::Invite => self.state == ClientTransactionState::Calling,
            ClientTransactionKind::NonInvite => matches!(
                self.state,
                ClientTransactionState::Trying | ClientTransactionState::Proceeding
            ),
        }
    }

    pub(super) fn state(&self) -> ClientTransactionState {
        self.state
    }
}
use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::Notify;
