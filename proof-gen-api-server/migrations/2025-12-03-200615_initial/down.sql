-- Drop continuity_proofs indices
DROP INDEX IF EXISTS continuity_chain_key_header_number;
DROP INDEX IF EXISTS continuity_created_at;

-- Drop merkle proofs indices
DROP INDEX IF EXISTS merkle_unique_null_tx_index;
DROP INDEX IF EXISTS merkle_unique_nonnull_tx_index;
DROP INDEX IF EXISTS merkle_by_tx_hash;
DROP INDEX IF EXISTS merkle_by_created_at;

-- Drop the table for proofs
DROP TABLE IF EXISTS merkle_proofs;
DROP TABLE IF EXISTS continuity_proofs;