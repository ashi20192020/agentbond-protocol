use agentbond_types::{ProtocolEvent, ProtocolEventKind};
use pinocchio::address::Address;

use crate::error::ProgramResult;

pub fn emit(
    kind: ProtocolEventKind,
    subject: &Address,
    actor: &Address,
    amount: u64,
    timestamp: i64,
) -> ProgramResult {
    let event = ProtocolEvent {
        kind,
        subject: subject.to_bytes(),
        actor: actor.to_bytes(),
        amount,
        timestamp,
    };
    emit_bytes(&event.encode());
    Ok(())
}

fn emit_bytes(data: &[u8]) {
    #[cfg(any(target_os = "solana", target_arch = "bpf"))]
    {
        // sol_log_data expects a pointer to an array of byte slices.
        let fields: [&[u8]; 1] = [data];
        unsafe {
            pinocchio::syscalls::sol_log_data(fields.as_ptr() as *const u8, fields.len() as u64);
        }
    }
    #[cfg(not(any(target_os = "solana", target_arch = "bpf")))]
    {
        let _ = data;
    }
}
