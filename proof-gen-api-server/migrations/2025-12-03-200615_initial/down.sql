-- Drop continuity_proofs indices
DROP INDEX IF EXISTS continuity_chain_key_header_number;
DROP INDEX IF EXISTS continuity_created_at;

-- Drop the table for proofs
DROP TABLE IF EXISTS continuity_proofs;