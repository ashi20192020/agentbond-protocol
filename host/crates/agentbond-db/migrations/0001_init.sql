-- AgentBond Milestone 4 schema (append-only).

CREATE TABLE indexer_slots (
    slot BIGINT PRIMARY KEY CHECK (slot >= 0),
    parent_slot BIGINT CHECK (parent_slot IS NULL OR parent_slot >= 0),
    status TEXT NOT NULL CHECK (status IN ('processed', 'confirmed', 'finalized', 'dead')),
    block_time TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE raw_account_versions (
    address BYTEA NOT NULL CHECK (octet_length(address) = 32),
    slot BIGINT NOT NULL CHECK (slot >= 0),
    write_version BIGINT NOT NULL CHECK (write_version >= 0),
    owner BYTEA CHECK (owner IS NULL OR octet_length(owner) = 32),
    lamports NUMERIC(20,0) NOT NULL CHECK (lamports >= 0),
    executable BOOLEAN NOT NULL DEFAULT FALSE,
    data BYTEA,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    commitment TEXT NOT NULL CHECK (commitment IN ('processed', 'confirmed', 'finalized')),
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (address, slot, write_version)
);

CREATE INDEX raw_account_versions_slot_idx ON raw_account_versions (slot);

CREATE TABLE raw_protocol_events (
    signature VARCHAR(128) NOT NULL CHECK (char_length(signature) BETWEEN 64 AND 128),
    event_index INT NOT NULL CHECK (event_index >= 0 AND event_index < 256),
    slot BIGINT NOT NULL CHECK (slot >= 0),
    program_id BYTEA NOT NULL CHECK (octet_length(program_id) = 32),
    kind SMALLINT NOT NULL CHECK (kind BETWEEN 1 AND 17),
    subject BYTEA NOT NULL CHECK (octet_length(subject) = 32),
    actor BYTEA NOT NULL CHECK (octet_length(actor) = 32),
    amount NUMERIC(20,0) NOT NULL CHECK (amount >= 0),
    event_timestamp TIMESTAMPTZ NOT NULL,
    commitment TEXT NOT NULL CHECK (commitment IN ('processed', 'confirmed', 'finalized')),
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (signature, event_index)
);

CREATE INDEX raw_protocol_events_slot_idx ON raw_protocol_events (slot);
CREATE INDEX raw_protocol_events_subject_idx ON raw_protocol_events (subject);

CREATE TABLE indexer_checkpoints (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    finalized_slot BIGINT NOT NULL CHECK (finalized_slot >= 0),
    processed_slot BIGINT NOT NULL CHECK (processed_slot >= 0),
    yellowstone_resume BYTEA CHECK (yellowstone_resume IS NULL OR octet_length(yellowstone_resume) <= 4096),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO indexer_checkpoints (id, finalized_slot, processed_slot)
VALUES (1, 0, 0);

CREATE TABLE ingestion_gaps (
    id BIGSERIAL PRIMARY KEY,
    from_slot BIGINT NOT NULL CHECK (from_slot >= 0),
    to_slot BIGINT NOT NULL CHECK (to_slot >= from_slot),
    status TEXT NOT NULL CHECK (status IN ('open', 'repairing', 'repaired', 'failed')),
    attempts INT NOT NULL DEFAULT 0 CHECK (attempts >= 0 AND attempts < 1000),
    last_error VARCHAR(512),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX ingestion_gaps_range_uidx ON ingestion_gaps (from_slot, to_slot);

CREATE TABLE proj_config (
    address BYTEA PRIMARY KEY CHECK (octet_length(address) = 32),
    as_of_slot BIGINT NOT NULL CHECK (as_of_slot >= 0),
    paused BOOLEAN NOT NULL,
    admin BYTEA NOT NULL CHECK (octet_length(admin) = 32),
    genesis_hash BYTEA NOT NULL CHECK (octet_length(genesis_hash) = 32),
    allowed_mint BYTEA NOT NULL CHECK (octet_length(allowed_mint) = 32),
    token_program BYTEA NOT NULL CHECK (octet_length(token_program) = 32),
    mint_decimals SMALLINT NOT NULL CHECK (mint_decimals BETWEEN 0 AND 18),
    min_provider_bond NUMERIC(20,0) NOT NULL CHECK (min_provider_bond >= 0),
    challenge_duration_seconds BIGINT NOT NULL CHECK (challenge_duration_seconds >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE proj_providers (
    address BYTEA PRIMARY KEY CHECK (octet_length(address) = 32),
    as_of_slot BIGINT NOT NULL CHECK (as_of_slot >= 0),
    authority BYTEA NOT NULL CHECK (octet_length(authority) = 32),
    status TEXT NOT NULL CHECK (status IN ('Active', 'Inactive')),
    execution_key_count SMALLINT NOT NULL CHECK (execution_key_count BETWEEN 0 AND 4),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX proj_providers_authority_idx ON proj_providers (authority);

CREATE TABLE proj_provider_bonds (
    address BYTEA PRIMARY KEY CHECK (octet_length(address) = 32),
    as_of_slot BIGINT NOT NULL CHECK (as_of_slot >= 0),
    authority BYTEA NOT NULL CHECK (octet_length(authority) = 32),
    mint BYTEA NOT NULL CHECK (octet_length(mint) = 32),
    deposited NUMERIC(20,0) NOT NULL CHECK (deposited >= 0),
    locked NUMERIC(20,0) NOT NULL CHECK (locked >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX proj_provider_bonds_authority_idx ON proj_provider_bonds (authority);

CREATE TABLE proj_jobs (
    address BYTEA PRIMARY KEY CHECK (octet_length(address) = 32),
    as_of_slot BIGINT NOT NULL CHECK (as_of_slot >= 0),
    buyer BYTEA NOT NULL CHECK (octet_length(buyer) = 32),
    provider BYTEA NOT NULL CHECK (octet_length(provider) = 32),
    mint BYTEA NOT NULL CHECK (octet_length(mint) = 32),
    token_program BYTEA NOT NULL CHECK (octet_length(token_program) = 32),
    amount NUMERIC(20,0) NOT NULL CHECK (amount >= 0),
    job_nonce NUMERIC(20,0) NOT NULL CHECK (job_nonce >= 0),
    state TEXT NOT NULL CHECK (state IN (
        'Created', 'Funded', 'Accepted', 'Submitted', 'Challenged',
        'Settled', 'Refunded', 'Expired', 'Slashed', 'Closed'
    )),
    fund_deadline TIMESTAMPTZ NOT NULL,
    accept_deadline TIMESTAMPTZ NOT NULL,
    work_deadline TIMESTAMPTZ NOT NULL,
    auto_settle_deadline TIMESTAMPTZ NOT NULL,
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash) = 32),
    receipt_digest BYTEA NOT NULL CHECK (octet_length(receipt_digest) = 32),
    locked_bond NUMERIC(20,0) NOT NULL CHECK (locked_bond >= 0),
    mint_decimals SMALLINT NOT NULL CHECK (mint_decimals BETWEEN 0 AND 18),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX proj_jobs_buyer_idx ON proj_jobs (buyer);
CREATE INDEX proj_jobs_provider_idx ON proj_jobs (provider);
CREATE INDEX proj_jobs_state_idx ON proj_jobs (state);

CREATE TABLE proj_challenges (
    address BYTEA PRIMARY KEY CHECK (octet_length(address) = 32),
    as_of_slot BIGINT NOT NULL CHECK (as_of_slot >= 0),
    job BYTEA NOT NULL CHECK (octet_length(job) = 32),
    buyer BYTEA NOT NULL CHECK (octet_length(buyer) = 32),
    reason_hash BYTEA NOT NULL CHECK (octet_length(reason_hash) = 32),
    bond_amount NUMERIC(20,0) NOT NULL CHECK (bond_amount = 0),
    deadline TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('Open', 'Resolved')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX proj_challenges_job_idx ON proj_challenges (job);

CREATE TABLE x402_challenges (
    memo VARCHAR(32) PRIMARY KEY CHECK (memo ~ '^[0-9a-f]{32}$'),
    service_id VARCHAR(64) NOT NULL CHECK (char_length(service_id) BETWEEN 1 AND 64),
    resource_url VARCHAR(512) NOT NULL CHECK (char_length(resource_url) BETWEEN 1 AND 512),
    description VARCHAR(256) NOT NULL CHECK (char_length(description) <= 256),
    merchant VARCHAR(64) NOT NULL CHECK (char_length(merchant) BETWEEN 32 AND 64),
    asset VARCHAR(64) NOT NULL CHECK (char_length(asset) BETWEEN 32 AND 64),
    amount VARCHAR(40) NOT NULL CHECK (char_length(amount) BETWEEN 1 AND 40),
    network VARCHAR(64) NOT NULL CHECK (char_length(network) BETWEEN 1 AND 64),
    fee_payer VARCHAR(64) NOT NULL CHECK (char_length(fee_payer) BETWEEN 32 AND 64),
    input_digest VARCHAR(64) NOT NULL CHECK (input_digest ~ '^[0-9a-f]{64}$'),
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    max_timeout_seconds BIGINT NOT NULL CHECK (max_timeout_seconds > 0 AND max_timeout_seconds <= 86400),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX x402_challenges_expires_idx ON x402_challenges (expires_at);

CREATE TABLE x402_settlements (
    tx_digest VARCHAR(64) PRIMARY KEY CHECK (tx_digest ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK (state IN ('settling', 'settled', 'failed')),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    service_id VARCHAR(64) NOT NULL CHECK (char_length(service_id) BETWEEN 1 AND 64),
    resource_url VARCHAR(512) NOT NULL CHECK (char_length(resource_url) BETWEEN 1 AND 512),
    input_digest VARCHAR(64) NOT NULL CHECK (input_digest ~ '^[0-9a-f]{64}$'),
    challenge_memo VARCHAR(32) NOT NULL CHECK (challenge_memo ~ '^[0-9a-f]{32}$'),
    result_body JSONB,
    payment_response_header VARCHAR(8192),
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT x402_settlements_settled_fields CHECK (
        state <> 'settled'
        OR (result_body IS NOT NULL AND payment_response_header IS NOT NULL)
    )
);

CREATE INDEX x402_settlements_lease_idx ON x402_settlements (state, lease_expires_at);
