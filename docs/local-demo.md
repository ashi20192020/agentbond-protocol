# Local Milestone 4 demo

## 1. Start PostgreSQL

```bash
docker compose up -d postgres
```

Local credentials are demo-only (`agentbond` / `agentbond_local_only`).

## 2. Export environment

```bash
export AGENTBOND_DATABASE_URL=postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond
```

## 3. Migrate

```bash
cargo run --manifest-path host/Cargo.toml -p agentbond-indexer-app --bin agentbond-indexer -- migrate
```

## 4. Replay fixture

```bash
cargo run --manifest-path host/Cargo.toml -p agentbond-indexer-app --bin agentbond-indexer -- replay --fixture host/fixtures/indexer/lifecycle.json
```

Replay is idempotent.

## 5. Start gateway in local mock mode

```bash
cd host
AGENTBOND_USE_MOCK=1 cargo run -p agentbond-gateway -- config/example.config.json
```

For indexed routes with PostgreSQL (no mock payments memory):

```bash
AGENTBOND_DATABASE_URL=postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond \
  cargo run --manifest-path host/Cargo.toml -p agentbond-gateway -- host/config/example.config.json
```

Compose publishes PostgreSQL on host port **5433** so it does not clash with other local listeners on 5432.

## 6. Query indexed endpoints

```bash
curl -s localhost:8080/v1/index/status
curl -s 'localhost:8080/v1/index/jobs?limit=20'
curl -s localhost:8080/v1/index/providers
```

## 7. x402 restart-recovery

Run host PostgreSQL payment tests:

```bash
AGENTBOND_DATABASE_URL=postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond \
  cargo test --manifest-path host/Cargo.toml -p agentbond-db --test payments_pg -- --nocapture
```

## Stop or remove PostgreSQL

```bash
docker compose stop postgres
# remove container and named volume only if you intend to wipe data:
docker compose down
docker volume rm agentbond-protocol_agentbond_pg_data
```
