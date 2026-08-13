# AgentBond

AgentBond is a Rust-first Solana protocol for agent-to-agent service work. It is designed around onchain job escrow, signed work receipts, provider bonds, and a small job state machine.

This repository is early. It is not production-ready and has not been audited.

## Current status

**Milestone 3**

Milestone 3 adds the offchain AgentBond platform on top of the Milestone 2 Pinocchio escrow program:

- Rust SDK (`agentbond-sdk`) for PDAs, account decoding, instruction builders, receipts, and unsigned instruction plans
- Shared application use cases (`agentbond-app`)
- CLI (`agentbond-cli`)
- HTTP gateway (`agentbond-gateway`) that returns unsigned plans and never holds user private keys
- MCP stdio server (`agentbond-mcp`, `rmcp = 3.0.1`, protocol `2026-07-28`) that builds plans and does not sign or submit them
- Narrow x402 v2 exact-SVM resource-server adapter (`agentbond-payments`) for one paid demo route
- Local agent simulator (`agentbond-sim`) using LiteSVM and a mock facilitator

Milestone 1 shared types remain the source of truth for layouts, receipts, and state rules. Milestone 2 onchain security tests remain in the root workspace.

The project is **not production-ready** and has **not been audited**.

## Workspaces and toolchains

The repository uses two Cargo workspaces so Agave SBF tooling stays on Rust 1.84 / edition 2021 while host services can use Rust 1.88 / edition 2024.

```text
.
├── crates/agentbond-types     # shared protocol types (root workspace)
├── programs/agentbond         # Pinocchio program + LiteSVM tests (root)
├── host/                      # separate host workspace (excluded from root)
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── config/                # example config + catalog only
│   ├── crates/
│   │   ├── agentbond-sdk/
│   │   ├── agentbond-payments/
│   │   └── agentbond-app/
│   └── apps/
│       ├── agentbond-cli/
│       ├── agentbond-gateway/
│       ├── agentbond-mcp/
│       └── agentbond-sim/
├── Cargo.toml                 # root SBF workspace; exclude = ["host"]
├── LICENSE
└── README.md
```

| Workspace | Rust | Edition | Purpose |
|---|---|---|---|
| Root | 1.84 (Agave SBF) | 2021 | `agentbond-types`, onchain program, program tests |
| `host/` | 1.88+ | 2024 | SDK, CLI, gateway, MCP, payments, simulator |

`agentbond-types` is consumed by the host workspace through a path dependency.

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

## SDK

`host/crates/agentbond-sdk` provides:

- PDA and ATA helpers matching onchain seeds
- Typed account decoding with owner/address/length/discriminator checks
- Instruction builders for every Milestone 2 instruction
- Transport-neutral `InstructionPlan` JSON (unsigned)
- Receipt validation, digest, Ed25519 verify instruction, and paired submit plan
- `ChainReader` RPC boundary with HTTP and in-memory mock implementations

Gateway and MCP return instruction plans. They never accept private keys and never submit escrow transactions.

## CLI examples

From `host/`:

```bash
cargo run -p agentbond-cli -- address config
cargo run -p agentbond-cli -- inspect job <JOB_PUBKEY> --rpc-url http://127.0.0.1:8899
cargo run -p agentbond-cli -- receipt create \
  --program-id <PK> --job <PK> --provider <PK> --buyer <PK> \
  --job-nonce 1 --amount 5000 --work-hash <64hex> --output-hash <64hex> \
  --issued-at <unix> --expires-at <unix> \
  --execution-keypair ./exec.json --json
cargo run -p agentbond-cli -- plan create-job --buyer <PK> --provider <PK> --job-nonce 1 --amount 5000 --json
cargo run -p agentbond-cli -- send --rpc-url http://127.0.0.1:8899 --payer-keypair ./payer.json --signer ./other.json --plan ./plan.json --yes
```

Plan subcommands cover every protocol instruction:

`initialize-config`, `set-paused`, `register-provider`, `add-execution-key`, `revoke-execution-key`, `deposit-bond`, `withdraw-bond`, `create-job`, `fund-job`, `accept-job`, `submit-receipt`, `accept-work`, `challenge-work`, `resolve-timeout`, `expire-unfunded`, `expire-unaccepted`, `slash-bond`, `close-job`.

Plans reuse `agentbond-sdk` builders (no duplicated account ordering). Human and `--json` output are supported. `receipt create` requires every receipt field or a complete input file; it never inserts hidden defaults.

Send flow (bounded RPC, no fake blockhash):

1. `getGenesisHash` → detect cluster; reject mainnet unless `--allow-mainnet` (genesis hash, not RPC URL text)
2. `getLatestBlockhash` → build transaction
3. Confirm required signers → `simulateTransaction` → stop on failure
4. Single `sendTransaction` → poll `getSignatureStatuses` to a fixed deadline

Loaded plans allow only AgentBond instructions, plus an Ed25519 precompile immediately before `SubmitReceipt`. `plan.program_id` must match the configured program. Expired plans and missing signers are rejected. Before `--yes`, the CLI prints network, program ID, action, mint, amount, and required signers.

Additional send rules:

- No default private-key path; Solana CLI default keypair is never auto-used
- `--yes` required for non-interactive submission
- No indefinite signing or submission retries

Offline WireMock tests cover the full JSON-RPC send path (`getGenesisHash` → clock → `getLatestBlockhash` → `simulateTransaction` → `sendTransaction` → `getSignatureStatuses`), including simulation-before-submit ordering, negative cases (sim failure, bad program, missing signer, expired plan, mainnet genesis guard, send/confirm errors, bounded confirmation timeout), plus an `assert_cmd` CLI process test against the mock RPC.

## Gateway endpoints

```text
GET  /health/live
GET  /health/ready
GET  /v1/services
GET  /v1/services/{service_id}
GET  /v1/providers/{address}
GET  /v1/jobs/{address}

POST /v1/plans/jobs/create
POST /v1/plans/jobs/fund
POST /v1/plans/jobs/accept
POST /v1/plans/jobs/submit-receipt
POST /v1/plans/jobs/accept-work
POST /v1/plans/jobs/challenge
POST /v1/plans/jobs/resolve-timeout

POST /v1/x402/services/{service_id}/invoke
```

Production gateway uses HTTP RPC and the configured facilitator. For local smoke with mock chain/facilitator:

```bash
cd host
AGENTBOND_USE_MOCK=1 cargo run -p agentbond-gateway -- config/example.config.json
```

Config requires `x402_fee_payer` (public key). RPC and facilitator URLs must not embed credentials. Responses include `x-request-id`; structured errors expose a stable code, safe message, and request id (no stack traces).

Gateway HTTP tests cover exact success for every plan route (action, program ID, instruction count, signers, request id), boundary failures (malformed JSON, bad keys/numbers, missing accounts, ineligible timeout, RPC failure, body limit, request timeout), and the x402 matrix (402/`PAYMENT-REQUIRED`, success, exact retry, concurrent settle-once, different-input replay, verify/settle/timeout failures, rejected payload never reaches the facilitator). Escrow plan routes never call the facilitator.

The gateway never holds user private keys. The x402 demo route never builds AgentBond escrow plans.

## MCP setup

Production MCP validates config and uses `HttpChainReader` by default. Logging goes to stderr so stdio framing stays intact.

```bash
cd host
cargo run -p agentbond-mcp -- config/example.config.json
```

Explicit local mock mode only:

```bash
AGENTBOND_USE_MOCK=1 cargo run -p agentbond-mcp -- config/example.config.json
```

Tools: `discover_services`, `inspect_provider`, `inspect_job`, `build_create_job`, `build_fund_job`, `build_submit_receipt`, `build_accept_work`, `build_challenge`, `build_timeout_resolution`.

JSON Schema types match request fields (addresses/hashes as strings; nonces/amounts/timestamps as integers; booleans; structured receipt). MCP tools build unsigned instruction plans only. They do not sign or submit and do not accept private keys. Protocol version: `2026-07-28`.

Host tests cover a real RMCP client/server duplex transport (initialize `2026-07-28`, tool listing, every published tool, numeric JSON args, structured tool errors, unknown-tool protocol errors, unsigned plan results, schema private-key bans, stderr logging). Direct dispatch helpers remain for unit checks only.

## x402 paid-demo flow

The x402 code is a **narrow AgentBond resource-server adapter** for `scheme = exact` on a configured Solana network. It is **not** an official or complete Rust x402 SDK. AgentBond does not ship a custom facilitator.

Exact SVM requirements include typed `extra.feePayer` (camelCase), plus optional `memo`, `recentBlockhash`, and `lastValidBlockHeight`. Each payment challenge issues a unique ≥16-byte hex memo bound to service, resource URL, merchant, asset, amount, network, input digest, and issuance time. Challenges use a bounded TTL store; there is no global `requirements_issued_at` that expires unrelated payments.

`POST /v1/x402/services/{service_id}/invoke`:

1. Missing `PAYMENT-SIGNATURE` → HTTP 402 with `PAYMENT-REQUIRED`
2. Decode and validate the payment header (version, resource, scheme, network, asset, amount, `payTo`, timeouts, `extra.feePayer`, challenge memo, single Base64 transaction within Solana size limits)
3. Facilitator `verify` (readiness via facilitator `/supported` for `scheme=exact` and configured network)
4. Deterministic demo transform (hash/echo of input — not an AI model)
5. Facilitator `settle` behind an atomic settlement state machine keyed by transaction digest: `Unseen → Settling → Settled` (120s TTL, bounded capacity, targeted eviction)
6. HTTP 200 with resource body and `PAYMENT-RESPONSE`

Exact retry after success may return the cached identical response. Concurrent duplicates do not settle twice. Different binding for the same transaction is rejected. Persistent payment recovery belongs to Milestone 4.

Do not describe AgentBond escrow as x402 escrow. These rails are separate.

## Agent simulator

```bash
cargo build-sbf --manifest-path programs/agentbond/Cargo.toml --features bpf-entrypoint
cd host
cargo run -p agentbond-sim
```

Demonstrates offline:

1. Honest provider settlement
2. Provider timeout and buyer refund
3. Buyer challenge and timeout settlement
4. Admin slash
5. Receipt replay rejection
6. Local x402 402 → verify → settle → 200 with a mock facilitator

## Security boundaries

- Gateway and MCP never hold user private keys
- Gateway and MCP return unsigned plans only for escrow flows
- CLI signing requires explicit keypair paths and `--yes`
- Sensitive headers (`PAYMENT-*`, `Authorization`) are redacted from tracing
- Facilitator URL comes only from trusted configuration
- Receipts prove a registered key signed a payload; they do **not** prove AI correctness
- `SlashBond` remains centralized admin arbitration

## x402 versus AgentBond escrow

| Rail | Role |
|---|---|
| AgentBond escrow | Onchain job state machine, bonds, receipts, settle/refund/slash |
| x402 exact (demo) | Immediate micropayment via facilitator verify/settle for a demo HTTP resource |

Do not treat a standard x402 transfer as AgentBond escrow funding.

## Receipt claim boundary

A work receipt proves that a registered key signed a specific payload.

It does **not** prove that an AI result is correct, complete, or high quality.

## Centralized challenge arbitration limitation

`SlashBond` is centralized MVP arbitration by the Config admin. A challenge is a subjective claim. Neither a challenge nor a slash objectively proves that work was incorrect.

## Legacy SPL Token limitation

Milestone 2/3 support only the legacy SPL Token program and one configured settlement mint. Token-2022, transfer hooks, confidential transfers, MPP, and SAS are out of scope for Milestone 3.

## Build and test

Root (SBF) workspace:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build-sbf --manifest-path programs/agentbond/Cargo.toml --features bpf-entrypoint
cargo test --workspace --all-features
```

Host workspace:

```bash
cargo fmt --manifest-path host/Cargo.toml --all -- --check
cargo clippy --manifest-path host/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path host/Cargo.toml --workspace --all-features
cargo run --manifest-path host/Cargo.toml -p agentbond-sim
```

Build the SBF binary before LiteSVM program tests or the simulator so they can load `target/deploy/agentbond.so`.

Current offline verification counts (no internet):

- Root workspace: **127** tests passed
- Host workspace: **49** tests passed
- Simulator: all **6** scenarios passed

Honest remaining limitations: no persistent x402 recovery, no live-cluster integration suite in CI, and the x402 adapter is not an official facilitator SDK.

## Program binary size

After `cargo build-sbf --features bpf-entrypoint`:

- `target/deploy/agentbond.so` = **151,480 bytes** (`wc -c`)

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
