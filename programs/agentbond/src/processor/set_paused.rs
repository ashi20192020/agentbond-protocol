use agentbond_types::ProtocolEventKind;
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{require_signer, require_writable};
use crate::events;
use crate::processor::helpers::{
    load_validated_config, next_account, now_ts, require_admin, save_config,
};

pub fn process(program_id: &Address, accounts: &[AccountView], paused: bool) -> ProgramResult {
    let mut accounts = accounts.iter();
    let admin = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;

    require_signer(admin)?;
    require_writable(config_account)?;

    let mut config = load_validated_config(program_id, config_account)?;
    require_admin(admin, &config)?;
    config.paused = paused;
    save_config(config_account, &config)?;

    events::emit(
        ProtocolEventKind::PauseChanged,
        config_account.address(),
        admin.address(),
        u64::from(paused),
        now_ts()?,
    )
}
