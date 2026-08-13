-- Persist staged decoded account projections for restart-safe finalization.
-- Extend gap statuses for honest partial repair.

CREATE TABLE staged_account_projections (
    address BYTEA NOT NULL CHECK (octet_length(address) = 32),
    slot BIGINT NOT NULL CHECK (slot >= 0),
    write_version BIGINT NOT NULL CHECK (write_version >= 0),
    kind TEXT NOT NULL CHECK (kind IN (
        'Config', 'Provider', 'ProviderBond', 'Job', 'Challenge', 'Tombstone'
    )),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (address, slot, write_version),
    CONSTRAINT staged_account_projections_payload_bound CHECK (
        pg_column_size(payload) <= 8192
    )
);

CREATE INDEX staged_account_projections_slot_idx
    ON staged_account_projections (slot);

ALTER TABLE ingestion_gaps
    DROP CONSTRAINT IF EXISTS ingestion_gaps_status_check;

ALTER TABLE ingestion_gaps
    ADD CONSTRAINT ingestion_gaps_status_check
    CHECK (status IN ('open', 'repairing', 'repaired', 'failed', 'partial'));
