-- Revert to old model
DROP INDEX IF EXISTS continuity_blocks_attestations;

-- Restore separate indexes (for rollback compatibility)
CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_attestations
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = TRUE AND is_checkpoint = FALSE;

CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_checkpoints
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = FALSE AND is_checkpoint = TRUE;

-- Restore constraint (will fail if any rows have both flags true)
ALTER TABLE continuity_blocks ADD CONSTRAINT continuity_blocks_flags_valid
    CHECK (NOT (is_attestation AND is_checkpoint));
