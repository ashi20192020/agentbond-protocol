#![cfg_attr(target_os = "solana", no_std)]

pub mod accounts;
pub mod constants;
pub mod ed25519;
pub mod entrypoint;
pub mod error;
pub mod events;
pub mod pda;
pub mod processor;
pub mod token;

pub use entrypoint::process_instruction;
pub use error::to_program_error;
pub use pda::{bond_address, challenge_address, config_address, job_address, provider_address, ID};
