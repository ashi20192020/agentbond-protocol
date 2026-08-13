use crate::error::ProtocolError;
use crate::state::JobState;

pub const EVENT_VERSION: u8 = 1;
pub const EVENT_ENCODED_LEN: usize = 82;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProtocolEventKind {
    ConfigInitialized = 1,
    PauseChanged = 2,
    ProviderRegistered = 3,
    ExecutionKeyAdded = 4,
    ExecutionKeyRevoked = 5,
    BondDeposited = 6,
    BondWithdrawn = 7,
    JobCreated = 8,
    JobFunded = 9,
    JobAccepted = 10,
    ReceiptSubmitted = 11,
    JobChallenged = 12,
    JobSettled = 13,
    JobRefunded = 14,
    JobExpired = 15,
    JobSlashed = 16,
    JobClosed = 17,
}

impl ProtocolEventKind {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::ConfigInitialized),
            2 => Ok(Self::PauseChanged),
            3 => Ok(Self::ProviderRegistered),
            4 => Ok(Self::ExecutionKeyAdded),
            5 => Ok(Self::ExecutionKeyRevoked),
            6 => Ok(Self::BondDeposited),
            7 => Ok(Self::BondWithdrawn),
            8 => Ok(Self::JobCreated),
            9 => Ok(Self::JobFunded),
            10 => Ok(Self::JobAccepted),
            11 => Ok(Self::ReceiptSubmitted),
            12 => Ok(Self::JobChallenged),
            13 => Ok(Self::JobSettled),
            14 => Ok(Self::JobRefunded),
            15 => Ok(Self::JobExpired),
            16 => Ok(Self::JobSlashed),
            17 => Ok(Self::JobClosed),
            _ => Err(ProtocolError::InvalidAccountData),
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolEvent {
    pub kind: ProtocolEventKind,
    pub subject: [u8; 32],
    pub actor: [u8; 32],
    pub amount: u64,
    pub timestamp: i64,
}

impl ProtocolEvent {
    pub fn encode(&self) -> [u8; EVENT_ENCODED_LEN] {
        let mut out = [0u8; EVENT_ENCODED_LEN];
        out[0] = EVENT_VERSION;
        out[1] = self.kind.as_u8();
        out[2..34].copy_from_slice(&self.subject);
        out[34..66].copy_from_slice(&self.actor);
        out[66..74].copy_from_slice(&self.amount.to_le_bytes());
        out[74..82].copy_from_slice(&self.timestamp.to_le_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() != EVENT_ENCODED_LEN {
            return Err(ProtocolError::InvalidAccountLength);
        }
        if data[0] != EVENT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        let mut subject = [0u8; 32];
        let mut actor = [0u8; 32];
        subject.copy_from_slice(&data[2..34]);
        actor.copy_from_slice(&data[34..66]);
        let amount = u64::from_le_bytes(
            data[66..74]
                .try_into()
                .map_err(|_| ProtocolError::InvalidAccountData)?,
        );
        let timestamp = i64::from_le_bytes(
            data[74..82]
                .try_into()
                .map_err(|_| ProtocolError::InvalidAccountData)?,
        );
        Ok(Self {
            kind: ProtocolEventKind::from_u8(data[1])?,
            subject,
            actor,
            amount,
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn terminal_event_mapping() {
        assert_eq!(
            ProtocolEventKind::for_terminal_state(JobState::Settled),
            Some(ProtocolEventKind::JobSettled)
        );
        assert_eq!(
            ProtocolEventKind::for_terminal_state(JobState::Created),
            None
        );
    }

    #[test]
    fn golden_event_vector() {
        let event = ProtocolEvent {
            kind: ProtocolEventKind::JobFunded,
            subject: [7u8; 32],
            actor: [8u8; 32],
            amount: 1_000_000,
            timestamp: 1_700_000_000,
        };
        let encoded = event.encode();
        assert_eq!(encoded.len(), 82);
        assert_eq!(encoded[0], 1);
        assert_eq!(encoded[1], ProtocolEventKind::JobFunded.as_u8());
        assert_eq!(ProtocolEvent::decode(&encoded).expect("decode"), event);
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..120)) {
            let _ = ProtocolEvent::decode(&data);
        }
    }
}
