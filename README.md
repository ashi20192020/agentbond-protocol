# AgentBond

AgentBond is a Rust-first Solana protocol for agent-to-agent service work. It is designed around onchain job escrow, signed work receipts, provider bonds, and a small job state machine.

This repository is early. It is not production-ready and has not been audited.

## Current status

**Milestone 2**

Milestone 2 replaces the Milestone 1 dispatch shell with a working Pinocchio escrow program. It supports configuration, provider registration, bonds, job funding, signed receipt submission, settlement, refunds, challenges, admin slash, and terminal account closure.

Milestone 1 shared types remain the source of truth for layouts, receipts, and state rules.

The Milestone 2 security suite is extensive (host codecs, Ed25519 layout parsing, and LiteSVM integration tests) but is **not an audit**.

## Implemented onchain instructions

- `InitializeConfig`, `SetPaused`
- `RegisterProvider`, `AddExecutionKey`, `RevokeExecutionKey`
- `DepositBond`, `WithdrawBond`
- `CreateJob`, `FundJob`, `AcceptJob`
- `SubmitReceipt` (Ed25519 precompile + Instructions sysvar)
- `AcceptWork`, `ChallengeWork`
- `ResolveTimeoutSettle`, `ResolveTimeoutRefund`
- `ExpireUnfunded`, `ExpireUnaccepted`
- `SlashBond`, `CloseJob`

## Account model

| Account | Seeds | Notes |
|---|---|---|
| Config | `["config"]` | Singleton; read-only on common job ops |
| Provider | `["provider", authority]` | Up to 4 execution keys |
| ProviderBond | `["bond", authority, mint]` | Tracks deposited/locked; vault is ATA(bond PDA) |
| Job | `["job", buyer, provider, nonce_le]` | Job PDA is escrow ATA authority |
| Challenge | `["challenge", job]` | `bond_amount` must remain 0 in M2 |

Escrow token account: ATA(job PDA, configured mint, legacy Token Program).

## Job state flow

```text
Created -> Funded -> Accepted -> Submitted -> Settled
                                  |            ^
                                  +-> Challenged -> Settled / Refunded / Slashed
Created -> Expired
Funded / Accepted -> Refunded (timeouts)
```

## Security invariants

- Principal accounting always uses `job.amount`.
- Unsolicited escrow dust is returned to the buyer and cannot inflate principal.
- Checked arithmetic for amounts and timestamps.
- Pause blocks register/deposit/create/fund/accept; it does not block submit, settle, refund, withdraw unlocked bond, or close.
- Legacy SPL Token only (`Tokenkeg...`). Token-2022 is rejected.
- SubmitReceipt requires the Ed25519 instruction immediately before AgentBond and validates the full 334-byte receipt message.
- Slash is admin-only while Challenged and before the challenge deadline.
- Config is not writable on common job operations.

## Legacy SPL Token limitation

Milestone 2 supports only the legacy SPL Token program and one configured settlement mint. Token-2022, transfer hooks, confidential transfers, and arbitrary mints are out of scope.

## Centralized challenge arbitration limitation

`SlashBond` is centralized MVP arbitration by the Config admin. A challenge is a subjective claim. Neither a challenge nor a slash objectively proves that work was incorrect.

## Receipt claim boundary

A work receipt proves that a registered key signed a specific payload.

It does **not** prove that an AI result is correct, complete, or high quality.

## Payment rails

x402 or MPP micropayments and AgentBond onchain escrow are separate payment rails.

- Micropayments settle immediately to a merchant token account.
- Expensive jobs use AgentBond escrow and the job state machine.

Do not treat a standard x402 transfer as AgentBond escrow funding.

## Workspace structure

```text
.
├── crates/agentbond-types   # shared protocol types and codecs
├── programs/agentbond       # Pinocchio program + LiteSVM tests
├── Cargo.toml
├── LICENSE
└── README.md
```

## Build and test

Host checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build-sbf --manifest-path programs/agentbond/Cargo.toml --features bpf-entrypoint
cargo test --workspace --all-features
```

Build the SBF binary before integration tests so LiteSVM can load `target/deploy/agentbond.so`.

Latest Milestone 2 verification counted **127** workspace tests (`cargo test --workspace --all-features`), including **77** LiteSVM integration tests under `programs/agentbond/tests/`.

## Program binary size

After `cargo build-sbf --features bpf-entrypoint`:

- `target/deploy/agentbond.so` = **145,976 bytes** (`wc -c`, Milestone 2 completion run)

## Compute-unit results

Measured once via `programs/agentbond/tests/compute_units.rs` on LiteSVM (values can vary slightly across runs):

| Instruction | Measured CU |
|---|---|
| FundJob | 18,798 |
| AcceptJob | 8,810 |
| SubmitReceipt | 48,159 |
| AcceptWork | 22,330 |
| ChallengeWork | 8,090 |
| ResolveTimeoutSettle (Submitted) | 17,856 |
| ResolveTimeoutSettle (Challenged) | 24,364 |
| ResolveTimeoutRefund (Funded) | 17,704 |
| ResolveTimeoutRefund (Accepted) | 22,215 |
| SlashBond | 42,686 |

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
