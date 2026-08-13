# AgentBond

AgentBond is a Rust-first Solana protocol for agent-to-agent service work. It is designed around onchain job escrow, signed work receipts, provider bonds, and a small job state machine.

This repository is early. It is not production-ready and has not been audited.

## Current status

**Milestone 1**

Milestone 1 provides the shared protocol foundation and a minimal Pinocchio program shell. It does not move tokens, settle jobs, or expose network services.

Milestone 1 contains:

- Cargo workspace
- Shared protocol types in `crates/agentbond-types`
- Canonical `AgentBondWorkReceiptV1` codec and SHA-256 digest helpers
- Job state transition rules
- Fixed account layouts
- Instruction discriminators and parsing
- PDA seed constants and derivation helpers
- Minimal Pinocchio entrypoint with instruction dispatch scaffolding

## Workspace structure

```text
.
├── crates/agentbond-types   # shared protocol types and codecs
├── programs/agentbond       # Pinocchio program shell
├── Cargo.toml               # workspace manifest
├── LICENSE
└── README.md
```

## Build and test

Host checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Solana program build (requires the Solana/Agave SBF toolchain):

```bash
cargo build-sbf --manifest-path programs/agentbond/Cargo.toml --features bpf-entrypoint
```

## Receipt claim boundary

A work receipt proves that a registered key signed a specific payload.

It does **not** prove that an AI result is correct, complete, or high quality.

## Payment rails

x402 or MPP micropayments and AgentBond onchain escrow are separate payment rails.

- Micropayments settle immediately to a merchant token account.
- Expensive jobs use AgentBond escrow and the job state machine.

Do not treat a standard x402 transfer as AgentBond escrow funding.

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
