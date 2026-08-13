use crate::error::ProtocolError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum JobState {
    Created = 0,
    Funded = 1,
    Accepted = 2,
    Submitted = 3,
    Challenged = 4,
    Settled = 5,
    Refunded = 6,
    Expired = 7,
    Slashed = 8,
}

impl JobState {
    pub const fn from_u8(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Created),
            1 => Ok(Self::Funded),
            2 => Ok(Self::Accepted),
            3 => Ok(Self::Submitted),
            4 => Ok(Self::Challenged),
            5 => Ok(Self::Settled),
            6 => Ok(Self::Refunded),
            7 => Ok(Self::Expired),
            8 => Ok(Self::Slashed),
            _ => Err(ProtocolError::InvalidJobState),
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Settled | Self::Refunded | Self::Expired | Self::Slashed
        )
    }
}

pub const fn is_terminal(state: JobState) -> bool {
    state.is_terminal()
}

pub const fn validate_transition(from: JobState, to: JobState) -> Result<(), ProtocolError> {
    if from.as_u8() == to.as_u8() {
        return Err(ProtocolError::InvalidStateTransition);
    }
    if from.is_terminal() {
        return Err(ProtocolError::InvalidStateTransition);
    }

    let allowed = matches!(
        (from, to),
        (JobState::Created, JobState::Funded)
            | (JobState::Created, JobState::Expired)
            | (JobState::Funded, JobState::Accepted)
            | (JobState::Funded, JobState::Refunded)
            | (JobState::Accepted, JobState::Submitted)
            | (JobState::Accepted, JobState::Refunded)
            | (JobState::Submitted, JobState::Settled)
            | (JobState::Submitted, JobState::Challenged)
            | (JobState::Challenged, JobState::Settled)
            | (JobState::Challenged, JobState::Refunded)
            | (JobState::Challenged, JobState::Slashed)
    );

    if allowed {
        Ok(())
    } else {
        Err(ProtocolError::InvalidStateTransition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ALL_STATES: [JobState; 9] = [
        JobState::Created,
        JobState::Funded,
        JobState::Accepted,
        JobState::Submitted,
        JobState::Challenged,
        JobState::Settled,
        JobState::Refunded,
        JobState::Expired,
        JobState::Slashed,
    ];

    const ALLOWED: [(JobState, JobState); 11] = [
        (JobState::Created, JobState::Funded),
        (JobState::Created, JobState::Expired),
        (JobState::Funded, JobState::Accepted),
        (JobState::Funded, JobState::Refunded),
        (JobState::Accepted, JobState::Submitted),
        (JobState::Accepted, JobState::Refunded),
        (JobState::Submitted, JobState::Settled),
        (JobState::Submitted, JobState::Challenged),
        (JobState::Challenged, JobState::Settled),
        (JobState::Challenged, JobState::Refunded),
        (JobState::Challenged, JobState::Slashed),
    ];

    fn is_allowed(from: JobState, to: JobState) -> bool {
        ALLOWED.contains(&(from, to))
    }

    #[test]
    fn allowed_transitions_succeed() {
        for &(from, to) in &ALLOWED {
            validate_transition(from, to).expect("allowed transition should succeed");
        }
    }

    #[test]
    fn unlisted_transitions_fail() {
        for from in ALL_STATES {
            for to in ALL_STATES {
                if is_allowed(from, to) {
                    continue;
                }
                assert_eq!(
                    validate_transition(from, to),
                    Err(ProtocolError::InvalidStateTransition),
                    "unexpected success for {:?} -> {:?}",
                    from,
                    to
                );
            }
        }
    }

    #[test]
    fn self_transitions_fail() {
        for state in ALL_STATES {
            assert_eq!(
                validate_transition(state, state),
                Err(ProtocolError::InvalidStateTransition)
            );
        }
    }

    #[test]
    fn terminal_states_reject_every_transition() {
        for from in ALL_STATES {
            if !from.is_terminal() {
                continue;
            }
            for to in ALL_STATES {
                assert_eq!(
                    validate_transition(from, to),
                    Err(ProtocolError::InvalidStateTransition)
                );
            }
        }
    }

    #[test]
    fn terminal_classification() {
        assert!(is_terminal(JobState::Settled));
        assert!(is_terminal(JobState::Refunded));
        assert!(is_terminal(JobState::Expired));
        assert!(is_terminal(JobState::Slashed));
        assert!(!is_terminal(JobState::Created));
        assert!(!is_terminal(JobState::Funded));
        assert!(!is_terminal(JobState::Accepted));
        assert!(!is_terminal(JobState::Submitted));
        assert!(!is_terminal(JobState::Challenged));
    }

    #[test]
    fn from_u8_round_trip() {
        for state in ALL_STATES {
            assert_eq!(JobState::from_u8(state.as_u8()).expect("valid"), state);
        }
        assert_eq!(JobState::from_u8(9), Err(ProtocolError::InvalidJobState));
        assert_eq!(JobState::from_u8(255), Err(ProtocolError::InvalidJobState));
    }

    proptest! {
        #[test]
        fn property_transition_coverage(from_raw in 0u8..=8, to_raw in 0u8..=8) {
            let from = JobState::from_u8(from_raw).expect("from state");
            let to = JobState::from_u8(to_raw).expect("to state");
            let result = validate_transition(from, to);
            if is_allowed(from, to) {
                prop_assert_eq!(result, Ok(()));
            } else {
                prop_assert_eq!(result, Err(ProtocolError::InvalidStateTransition));
            }
        }

        #[test]
        fn property_invalid_state_byte_never_panics(value in any::<u8>()) {
            let _ = JobState::from_u8(value);
        }
    }
}
