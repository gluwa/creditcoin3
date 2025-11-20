-- Drop proofs indices
DROP INDEX IF EXISTS proofs_idx_chain_and_height;
DROP INDEX IF EXISTS proofs_idx_chain_height_and_tx_idx;

-- Drop the table for proofs
DROP TABLE IF EXISTS proofs;