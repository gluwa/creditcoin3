-- Drop proofs indices
DROP INDEX IF EXISTS proofs_unique_null_tx_index;
DROP INDEX IF EXISTS proofs_unique_nonnull_tx_index;

-- Drop the table for proofs
DROP TABLE IF EXISTS proofs;
DROP TABLE IF EXISTS example;