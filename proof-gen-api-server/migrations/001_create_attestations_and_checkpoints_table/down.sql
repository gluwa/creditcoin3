-- Drop partial unique indexes for (chain_key, header_number)
DROP INDEX IF EXISTS continuity_blocks_attestations;
DROP INDEX IF EXISTS continuity_blocks_checkpoints;
DROP INDEX IF EXISTS continuity_blocks_regular;

-- Drop complete index
DROP INDEX IF EXISTS continuity_blocks_complete;

-- Finally drop the table (this also removes the CHECK constraint)
DROP TABLE IF EXISTS continuity_blocks;
