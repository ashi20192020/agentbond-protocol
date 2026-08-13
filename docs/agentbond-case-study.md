# AgentBond case study

AgentBond is a Rust-first Solana protocol for agent-to-agent service work with onchain escrow, signed receipts, provider bonds, and a small job state machine. This write-up is for engineers and recruiters. The project is not audited and is not marketed as production-ready.

## 1. The problem

Autonomous agents need to hire other agents for work, hold funds in escrow, accept signed delivery claims, and resolve timeouts or disputes. Existing payment rails either move money immediately without job state, or require heavy custom onchain logic without a clear offchain operator surface.

## 2. Why two payment rails exist

AgentBond escrow is a job lifecycle with bonds, deadlines, receipts, settle/refund/slash.

x402 exact-SVM is a separate micropayment path for a paid HTTP demo resource through a facilitator.

They are not the same escrow. A standard x402 transfer does not fund an AgentBond job.

## 3. Trust boundaries

- Users keep private keys. Gateway and MCP never accept them.
- Plans are unsigned instruction bundles.
- Indexed PostgreSQL data is a read model only.
- Receipts prove a registered key signed a payload. They do not prove AI quality.

## 4. Onchain state machine

Jobs move through Created, Funded, Accepted, Submitted, Challenged, and terminal Settled/Refunded/Expired/Slashed/Closed states. Bonds lock during work. Timeouts and admin slash cover failure paths.

## 5. Signed receipt limitations

An Ed25519-signed 334-byte receipt binds program, job, provider, hashes, and deadlines. It is authorship evidence, not correctness verification. No TEE, SAS, or verifier voting is claimed.

## 6. MCP and unsigned-plan design

The MCP server builds plans over protocol `2026-07-28`. It lists tools, validates typed arguments, and returns structured unsigned plans. It does not sign or submit.

## 7. x402 replay protection

Payment challenges use cryptographically random memos bound to route and input digest. Settlements are keyed by transaction digest with `Unseen -> Settling -> Settled|Failed` and lease ownership. Exact retries return the stored result. Concurrent workers settle once locally.

## 8. Yellowstone ingestion

A production Yellowstone gRPC client (9.1.x, Apache-2.0 packages) streams slots, program accounts, and transactions behind a `ChainSource` trait. Subscribe requests resume from the stored finalized checkpoint when supported. Fixture replay keeps tests offline. Protocol `Program data:` lines are accepted only while AgentBond is the active invoke-stack program.

## 9. Fork-safe finalized projection

Processed account updates persist raw rows and staged decoded projections in PostgreSQL. Public projections and checkpoint advancement apply together when a slot finalizes. Restart between processed ingestion and finalization keeps staged work. Dead forks clean non-finalized staging only. Gap repair may restore events via bounded `getBlock` and leaves gaps `partial` until account coverage is reconciled.

## 10. PostgreSQL settlement recovery

Production gateway requires `AGENTBOND_DATABASE_URL` and already-applied migrations. Challenges and settlements persist across restarts. Lease expiry allows recovery after crashes. A remaining honest gap: facilitator-side exactly-once is outside AgentBond’s control.

## 11. Tests and measured compute units

Root program tests and LiteSVM CU measurements remain from earlier milestones. Host tests cover MCP transport, gateway boundaries, offline simulate-before-send, invoke-stack decoding, restart-safe projections, indexed API validation, metrics endpoints, and PostgreSQL payment recovery.

## 12. Honest limitations

- Not audited
- Centralized admin slash
- Legacy SPL Token only
- No Token-2022, MPP, SAS, confidential transfers, or TEE verification
- No live deployment automation
- Indexer lag means indexed reads are not a substitute for live plan validation
- Gap repair is not claimed complete without account reconciliation
- Persistent local settlement does not guarantee global exactly-once facilitator delivery

## 13. Recruiter-focused technical highlights

- Dual Rust workspaces for SBF 1.84 and host 1.88
- Pinocchio onchain program with explicit account codecs
- Transport-neutral unsigned instruction plans
- RMCP MCP server with real duplex transport tests
- Narrow unofficial x402 resource-server adapter with lease-based recovery
- Yellowstone gRPC indexer with durable staged projections and checkpoint resume
- Cursor-paginated finalized read APIs with strict validation
- Offline-first CI with pinned PostgreSQL 16.6, explicit migrate, SBF install, and size budget
