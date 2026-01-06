-- Create table for continuity blocks (unified attestations, checkpoints, and regular blocks)
CREATE TABLE IF NOT EXISTS continuity_blocks (
    id SERIAL PRIMARY KEY,
    -- Fine to use bigint here as chain_keys won't be larger than U32::Max
    chain_key BIGINT NOT NULL,
    -- Must use NUMERIC here, or else search logic will break for values > U32::Max
    header_number NUMERIC(20, 0) NOT NULL,
    digest VARCHAR(66) NOT NULL,
    is_attestation BOOLEAN NOT NULL,
    is_checkpoint BOOLEAN NOT NULL,
    -- Prevent invalid (true, true) state
    CONSTRAINT continuity_blocks_flags_valid
        CHECK (NOT (is_attestation AND is_checkpoint))
);

-- Partial unique indexes for each block type
-- 1) attestation only (true, false)
CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_attestations
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = TRUE AND is_checkpoint = FALSE;

-- 2) checkpoint only (false, true)
CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_checkpoints
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = FALSE AND is_checkpoint = TRUE;

-- 3) "regular" blocks (false, false)
CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_regular
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = FALSE AND is_checkpoint = FALSE;

-- 4) Complete index for block retrieval (non-unique, for range queries)
CREATE INDEX IF NOT EXISTS continuity_blocks_complete
ON continuity_blocks (chain_key, header_number);
