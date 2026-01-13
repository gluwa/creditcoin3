-- Migration: Allow checkpoints to be attestations and fix unique constraint
-- Checkpoints are a special type of attestation, so both flags should be TRUE

-- 1. Drop the constraint preventing both flags being true (must be first!)
ALTER TABLE continuity_blocks DROP CONSTRAINT IF EXISTS continuity_blocks_flags_valid;

-- 2. Handle case where both attestation (TRUE, FALSE) and checkpoint (FALSE, TRUE) exist
--    for the same (chain_key, header_number). Merge by updating the attestation row
--    to be a checkpoint, then delete the orphan checkpoint row.
UPDATE continuity_blocks att
SET is_checkpoint = TRUE
FROM continuity_blocks cp
WHERE att.chain_key = cp.chain_key
  AND att.header_number = cp.header_number
  AND att.is_attestation = TRUE AND att.is_checkpoint = FALSE
  AND cp.is_attestation = FALSE AND cp.is_checkpoint = TRUE;

DELETE FROM continuity_blocks
WHERE is_attestation = FALSE AND is_checkpoint = TRUE
  AND EXISTS (
    SELECT 1 FROM continuity_blocks att
    WHERE att.chain_key = continuity_blocks.chain_key
      AND att.header_number = continuity_blocks.header_number
      AND att.is_attestation = TRUE
  );

-- 3. Transform remaining standalone checkpoints from old format (FALSE, TRUE) to new format (TRUE, TRUE)
UPDATE continuity_blocks
SET is_attestation = TRUE
WHERE is_attestation = FALSE AND is_checkpoint = TRUE;

-- 4. Drop old partial unique indexes (from migration 001)
DROP INDEX IF EXISTS continuity_blocks_attestations;
DROP INDEX IF EXISTS continuity_blocks_checkpoints;

-- 5. Create unified unique index for ALL attestations (prevents duplicates regardless of checkpoint flag)
-- This handles the race condition where CheckpointReached arrives before BlockAttested
CREATE UNIQUE INDEX IF NOT EXISTS continuity_blocks_attestations
ON continuity_blocks (chain_key, header_number)
WHERE is_attestation = TRUE;

-- Note: continuity_blocks_regular index already exists from migration 001
