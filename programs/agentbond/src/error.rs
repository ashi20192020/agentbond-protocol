use agentbond_types::ProtocolError;
use pinocchio::error::ProgramError;

pub fn to_program_error(error: ProtocolError) -> ProgramError {
    ProgramError::Custom(error.code())
}

pub fn from_protocol(error: ProtocolError) -> ProgramError {
    to_program_error(error)
}

pub type ProgramResult = pinocchio::error::ProgramResult;

#[inline(always)]
pub fn fail(error: ProtocolError) -> ProgramError {
    from_protocol(error)
}
