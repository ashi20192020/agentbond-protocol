/// Protocol-level errors shared by host code and the onchain program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProtocolError {
    InvalidInstructionData = 1,
    EmptyInstructionData = 2,
    UnknownInstruction = 3,
    InstructionNotImplemented = 4,
    InvalidInstructionLength = 5,
    InvalidAccountData = 6,
    InvalidAccountDiscriminator = 7,
    UnsupportedAccountVersion = 8,
    InvalidAccountLength = 9,
    InvalidJobState = 10,
    InvalidStateTransition = 11,
    InvalidReceiptLength = 12,
    InvalidReceiptDomain = 13,
    UnsupportedReceiptVersion = 14,
    InvalidReceiptTimestamps = 15,
    InvalidBoolean = 16,
    InvalidProviderStatus = 17,
    IntegerOverflow = 18,
}

impl ProtocolError {
    pub const fn code(self) -> u32 {
        self as u32
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInstructionData => "invalid instruction data",
            Self::EmptyInstructionData => "empty instruction data",
            Self::UnknownInstruction => "unknown instruction",
            Self::InstructionNotImplemented => "instruction not implemented",
            Self::InvalidInstructionLength => "invalid instruction length",
            Self::InvalidAccountData => "invalid account data",
            Self::InvalidAccountDiscriminator => "invalid account discriminator",
            Self::UnsupportedAccountVersion => "unsupported account layout version",
            Self::InvalidAccountLength => "invalid account length",
            Self::InvalidJobState => "invalid job state",
            Self::InvalidStateTransition => "invalid state transition",
            Self::InvalidReceiptLength => "invalid receipt length",
            Self::InvalidReceiptDomain => "invalid receipt domain",
            Self::UnsupportedReceiptVersion => "unsupported receipt version",
            Self::InvalidReceiptTimestamps => "invalid receipt timestamps",
            Self::InvalidBoolean => "invalid boolean byte",
            Self::InvalidProviderStatus => "invalid provider status",
            Self::IntegerOverflow => "integer overflow",
        }
    }
}

impl core::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProtocolError {}
