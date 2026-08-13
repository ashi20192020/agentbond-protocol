# AgentBond

AgentBond is a Rust-first Solana protocol for agent-to-agent service work. It is designed around onchain job escrow, signed work receipts, provider bonds, and a small job state machine.

This repository is early. It is not production-ready and has not been audited.

## Current status

**Milestone 4**

Milestone 4 adds the durable read and recovery layer on top of Milestones 1–3:

- PostgreSQL migrations and repositories (`agentbond-db`)
- Yellowstone gRPC + fixture indexer (`agentbond-indexer`, `agentbond-indexer` CLI)
- Fork-safe finalized projections (public reads are finalized-only)
- Persistent x402 challenge and settlement recovery (memory only with explicit mock mode)
- Indexed gateway APIs under `/v1/index/*`
- Metrics/readiness for the indexer, Compose PostgreSQL demo, CI, and portfolio docs

Earlier milestones remain: shared types, Pinocchio escrow program, SDK/CLI/gateway/MCP/simulator, and the narrow unofficial x402 adapter.

The project is **not production-ready** and has **not been audited**. Indexed data is a read model and cannot move funds. Receipts prove authorship, not correctness. Persistent local settlement does not guarantee global exactly-once facilitator behavior. SAS, MPP, Token-2022, confidential transfers, TEE verification, and live deployment remain out of scope.

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
│   │   ├── agentbond-app/
│   │   ├── agentbond-db/
│   │   └── agentbond-indexer/
│   ├── apps/
│   │   ├── agentbond-cli/
│   │   ├── agentbond-gateway/
│   │   ├── agentbond-mcp/
│   │   ├── agentbond-sim/
│   │   └── agentbond-indexer/
│   └── fixtures/indexer/
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

GET  /v1/index/status
GET  /v1/index/jobs
GET  /v1/index/jobs/{address}/history
GET  /v1/index/providers
GET  /v1/index/providers/{address}/activity
```

Indexed endpoints return finalized projections only, include `as_of_slot`, use cursor pagination (max limit 100), and never replace live `ChainReader` validation on plan routes.

Production gateway uses HTTP RPC, the configured facilitator, and PostgreSQL (`AGENTBOND_DATABASE_URL` required unless mock). For local smoke with mock chain/facilitator/memory payments:

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

Exact retry after success may return the cached identical response. Concurrent duplicates do not settle twice. Different binding for the same transaction is rejected. Production uses PostgreSQL-backed challenge and settlement stores with lease tokens; in-memory stores require explicit `AGENTBOND_USE_MOCK=1`. Local lease ownership does not guarantee global exactly-once facilitator behavior if a process crashes after remote settle and before database completion.

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

Milestone 2–4 support only the legacy SPL Token program and one configured settlement mint. Token-2022, transfer hooks, confidential transfers, MPP, and SAS remain out of scope.

## Indexer

```bash
# env: AGENTBOND_DATABASE_URL, AGENTBOND_YELLOWSTONE_URL, AGENTBOND_PROGRAM_ID, …
cargo run --manifest-path host/Cargo.toml -p agentbond-indexer-app --bin agentbond-indexer -- migrate
cargo run --manifest-path host/Cargo.toml -p agentbond-indexer-app --bin agentbond-indexer -- run
cargo run --manifest-path host/Cargo.toml -p agentbond-indexer-app --bin agentbond-indexer -- replay --fixture host/fixtures/indexer/lifecycle.json
```

Public projections are finalized-only. Processed updates may stage raw data. The database never authorizes token movement or signing. Yellowstone client packages used here are Apache-2.0; see [docs/third-party-licenses.md](docs/third-party-licenses.md).

Local PostgreSQL only (pin `postgres:16.6-alpine`):

```bash
docker compose up -d postgres
export AGENTBOND_DATABASE_URL=postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond
```

Stop with `docker compose stop postgres`. Remove the named volume only if you intend to wipe local data.

See [docs/local-demo.md](docs/local-demo.md), [docs/architecture.md](docs/architecture.md), and [docs/agentbond-case-study.md](docs/agentbond-case-study.md).

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

CI also runs PostgreSQL integration tests, fixture replay, and an SBF size budget of 180,000 bytes. No workflow step contacts mainnet, devnet, Yellowstone, or a live facilitator.

Current offline verification counts (no internet):

- Root workspace: **127** tests passed
- Host workspace: **61** tests passed
- Simulator: all **6** scenarios passed

Honest remaining limitations: no live-cluster Yellowstone suite in CI; the x402 adapter is not an official facilitator SDK; SAS/MPP/Token-2022/TEE/deployment remain future work.

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
