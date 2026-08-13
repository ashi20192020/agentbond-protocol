use crate::state::JobState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProtocolEvent {
    ConfigInitialized = 1,
    ProviderRegistered = 2,
    ExecutionKeyUpdated = 3,
    BondChanged = 4,
    JobCreated = 5,
    JobFunded = 6,
    JobAccepted = 7,
    JobSubmitted = 8,
    JobChallenged = 9,
    JobSettled = 10,
    JobRefunded = 11,
    JobExpired = 12,
    JobSlashed = 13,
    JobClosed = 14,
}

impl ProtocolEvent {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn for_terminal_state(state: JobState) -> Option<Self> {
        match state {
            JobState::Settled => Some(Self::JobSettled),
            JobState::Refunded => Some(Self::JobRefunded),
            JobState::Expired => Some(Self::JobExpired),
            JobState::Slashed => Some(Self::JobSlashed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_event_mapping() {
        assert_eq!(
            ProtocolEvent::for_terminal_state(JobState::Settled),
            Some(ProtocolEvent::JobSettled)
        );
        assert_eq!(ProtocolEvent::for_terminal_state(JobState::Created), None);
    }
}
