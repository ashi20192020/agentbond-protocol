mod common;

use agentbond_types::ProtocolError;
use common::{setup, token_2022_id, JOB_AMOUNT, MIN_BOND};
use solana_instruction::AccountMeta;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn deposit_and_multiple_deposits() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    env.deposit_bond(MIN_BOND);
    assert_eq!(env.read_bond().deposited, MIN_BOND);
    env.deposit_bond(MIN_BOND);
    assert_eq!(env.read_bond().deposited, MIN_BOND * 2);
    assert_eq!(env.read_bond().locked, 0);
}

#[test]
fn withdraw_partial_and_full_unlocked() {
    let mut env = setup();
    env.bootstrap_ready();
    let start = env.read_bond().deposited;
    env.withdraw_bond(100);
    assert_eq!(env.read_bond().deposited, start - 100);
    let left = env.read_bond().deposited;
    env.withdraw_bond(left);
    assert_eq!(env.read_bond().deposited, 0);
}

#[test]
fn withdraw_greater_than_deposited_and_unlocked() {
    let mut env = setup();
    env.bootstrap_ready();
    let deposited = env.read_bond().deposited;
    let ata = env.create_ata(&env.provider.pubkey());
    let vault = env.bond_vault();
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_withdraw_bond(deposited + 1, ata, vault)],
        &[],
        ProtocolError::InsufficientBond,
    );

    // Lock bond via accept, then withdraw unlocked only.
    let job = env.create_job(1);
    env.fund_job(&job);
    env.accept_job(&job);
    let bond = env.read_bond();
    assert_eq!(bond.locked, MIN_BOND);
    let unlocked = bond.deposited - bond.locked;
    env.send_err_code(
        &payer,
        &[env.ix_withdraw_bond(unlocked + 1, ata, vault)],
        &[],
        ProtocolError::InsufficientBond,
    );
    env.withdraw_bond(unlocked);
    assert_eq!(env.read_bond().locked, MIN_BOND);
    assert_eq!(env.read_bond().deposited, MIN_BOND);
}

#[test]
fn locked_bond_cannot_be_withdrawn() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    env.deposit_bond(MIN_BOND);
    let job = env.create_job(2);
    env.fund_job(&job);
    env.accept_job(&job);
    let ata = env.create_ata(&env.provider.pubkey());
    let vault = env.bond_vault();
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_withdraw_bond(1, ata, vault)],
        &[],
        ProtocolError::InsufficientBond,
    );
}

#[test]
fn locked_gt_deposited_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let deposited = env.read_bond().deposited;
    // locked / deposited are u64 LE at the end of ProviderBondAccount layout.
    env.write_bond_raw(|data| {
        let locked = deposited.saturating_add(1);
        data[108..116].copy_from_slice(&locked.to_le_bytes());
    });
    let job = env.create_job(3);
    env.fund_job(&job);
    let accept = env.ix_accept_job(&job);
    let payer = env.provider.insecure_clone();
    // Bond decode rejects locked > deposited before any mutation.
    env.send_err_code(&payer, &[accept], &[], ProtocolError::InvalidAccountData);
}

#[test]
fn wrong_bond_pda_vault_mint_token_program() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND * 2);
    let vault = env.ensure_bond_vault();
    let payer = env.provider.insecure_clone();

    let mut ix = env.ix_deposit_bond(MIN_BOND, ata, vault);
    ix.accounts[3] = AccountMeta::new(Pubkey::new_unique(), false);
    env.send_err_code(&payer, &[ix], &[], ProtocolError::InvalidPda);

    let mut ix = env.ix_deposit_bond(MIN_BOND, ata, vault);
    let stranger_ata = env.create_ata(&env.buyer.pubkey());
    ix.accounts[4] = AccountMeta::new(stranger_ata, false);
    env.send_err_code(
        &payer,
        &[ix],
        &[],
        ProtocolError::InvalidAssociatedTokenAccount,
    );

    let mut ix = env.ix_deposit_bond(MIN_BOND, ata, vault);
    ix.accounts[7] = AccountMeta::new_readonly(token_2022_id(), false);
    env.send_err_code(&payer, &[ix], &[], ProtocolError::InvalidTokenProgram);
}

#[test]
fn wrong_provider_authority_on_deposit() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("airdrop");
    let ata = env.create_ata(&stranger.pubkey());
    env.mint_to(&ata, MIN_BOND);
    let vault = env.ensure_bond_vault();
    let mut ix = env.ix_deposit_bond(MIN_BOND, ata, vault);
    ix.accounts[0] = AccountMeta::new(stranger.pubkey(), true);
    env.send_err_code(&stranger, &[ix], &[], ProtocolError::Unauthorized);
}

#[test]
fn wrong_token_account_owner_mint_on_withdraw() {
    let mut env = setup();
    env.bootstrap_ready();
    let payer = env.provider.insecure_clone();
    let vault = env.bond_vault();
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.send_err_code(
        &payer,
        &[env.ix_withdraw_bond(100, buyer_ata, vault)],
        &[],
        ProtocolError::InvalidTokenAccountAuthority,
    );
}

#[test]
fn frozen_and_delegated_token_accounts_rejected() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND * 2);
    let vault = env.ensure_bond_vault();
    env.force_frozen(&ata);
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_deposit_bond(MIN_BOND, ata, vault)],
        &[],
        ProtocolError::TokenAccountFrozen,
    );

    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND * 2);
    let vault = env.ensure_bond_vault();
    let delegate = Pubkey::new_unique();
    env.force_delegate(&ata, &delegate, 10);
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_deposit_bond(MIN_BOND, ata, vault)],
        &[],
        ProtocolError::InvalidTokenAccountAuthority,
    );
}

#[test]
fn direct_vault_donation_ignored_and_not_withdrawable() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    env.deposit_bond(MIN_BOND);
    let vault = env.bond_vault();
    let tracked = env.read_bond().deposited;
    env.mint_to(&vault, 777);
    assert_eq!(env.token_balance(&vault), tracked + 777);
    assert_eq!(env.read_bond().deposited, tracked);

    // Full tracked withdraw leaves donation dust in vault; further withdraw fails.
    env.withdraw_bond(tracked);
    assert_eq!(env.read_bond().deposited, 0);
    assert_eq!(env.token_balance(&vault), 777);
    let ata = env.create_ata(&env.provider.pubkey());
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_withdraw_bond(1, ata, vault)],
        &[],
        ProtocolError::InsufficientBond,
    );
}

#[test]
fn deposit_arithmetic_overflow_rejected() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    env.deposit_bond(MIN_BOND);
    let mut bond = env.read_bond();
    bond.deposited = u64::MAX;
    env.write_bond(&bond);
    // Keep vault solvent for transfer attempt; overflow happens after transfer on tracked add.
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND);
    let vault = env.bond_vault();
    // Top up vault so deposited tracking is already max; deposit tries checked_add.
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_deposit_bond(1, ata, vault)],
        &[],
        ProtocolError::MathOverflow,
    );
}

#[test]
fn wrong_mint_on_deposit() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let other_mint = Keypair::new();
    let rent = env.svm.minimum_balance_for_rent_exemption(82);
    let ixs = [
        solana_system_interface::instruction::create_account(
            &env.admin.pubkey(),
            &other_mint.pubkey(),
            rent,
            82,
            &TOKEN_PROGRAM_ID,
        ),
        spl_token::instruction::initialize_mint(
            &TOKEN_PROGRAM_ID,
            &other_mint.pubkey(),
            &env.mint_authority.pubkey(),
            None,
            6,
        )
        .expect("mint"),
    ];
    let admin = env.admin.insecure_clone();
    env.send_ok(&admin, &ixs, &[&other_mint]);
    let ata = get_associated_token_address(&env.provider.pubkey(), &other_mint.pubkey());
    let create = spl_associated_token_account_client::instruction::create_associated_token_account(
        &env.provider.pubkey(),
        &env.provider.pubkey(),
        &other_mint.pubkey(),
        &TOKEN_PROGRAM_ID,
    );
    let provider = env.provider.insecure_clone();
    env.send_ok(&provider, &[create], &[]);
    // Mint other tokens
    let mint_ix = spl_token::instruction::mint_to(
        &TOKEN_PROGRAM_ID,
        &other_mint.pubkey(),
        &ata,
        &env.mint_authority.pubkey(),
        &[],
        MIN_BOND,
    )
    .expect("mint_to");
    let mint_auth = env.mint_authority.insecure_clone();
    env.send_ok(&mint_auth, &[mint_ix], &[]);

    let vault = env.ensure_bond_vault();
    let mut ix = env.ix_deposit_bond(MIN_BOND, ata, vault);
    ix.accounts[6] = AccountMeta::new_readonly(other_mint.pubkey(), false);
    env.send_err_code(&provider, &[ix], &[], ProtocolError::InvalidMint);
    let _ = JOB_AMOUNT;
}
