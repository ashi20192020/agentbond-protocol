#![no_std]

pub mod entrypoint;
pub mod error;
pub mod pda;
pub mod processor;

pub use entrypoint::process_instruction;
pub use error::to_program_error;
pub use pda::{bond_address, challenge_address, config_address, job_address, provider_address, ID};
