# AgentBond architecture (Milestone 4)

## Workspaces

- Root workspace (Rust 1.84): onchain Pinocchio program and shared `agentbond-types`.
- Host workspace (Rust 1.88): SDK, CLI, gateway, MCP, payments, simulator, PostgreSQL (`agentbond-db`), indexer.

## Trust boundaries

- Onchain program is the authority for token movement and job state transitions.
- Gateway and MCP return unsigned plans only for escrow flows. They never hold user private keys.
- The PostgreSQL index is a read model. It cannot authorize transfers or signing.
- Public indexed APIs expose finalized projections only.
- x402 exact-SVM payments are a separate micropayment rail from AgentBond escrow.

## Ingestion

`ChainSource` abstracts Yellowstone gRPC and fixture replay.

Yellowstone production source:

- Configured via `AGENTBOND_YELLOWSTONE_URL`, optional `AGENTBOND_YELLOWSTONE_X_TOKEN`, `AGENTBOND_PROGRAM_ID`.
- Rejects embedded URL credentials.
- Requires TLS for non-loopback endpoints.
- Uses bounded reconnect backoff with jitter.
- Subscribes to program-owned accounts, slots, and relevant transactions.

## Fork-safe projection

1. Processed updates enter raw staging tables.
2. Only finalized slots update `proj_*` tables.
3. Finalizing a slot and advancing the checkpoint happen in one database transaction.
4. Dead slots delete only non-finalized staging rows.
5. Duplicate inserts are idempotent.
6. Newer `as_of_slot` values win; older writes do not replace newer projections.
7. Conflicting dead ancestry is a hard error.
8. Slot gaps are recorded; bounded RPC backfill may repair them later.

## Persistent x402 recovery

Challenge and settlement stores are async traits.

- Memory stores: tests and explicit `AGENTBOND_USE_MOCK=1`.
- PostgreSQL stores: production gateway when `AGENTBOND_DATABASE_URL` is set.

Settlement state machine: `Unseen -> Settling -> Settled | Failed` with lease tokens.

Honest crash window: a crash after facilitator settle and before DB completion can retry settlement; global exactly-once depends on facilitator idempotency.
