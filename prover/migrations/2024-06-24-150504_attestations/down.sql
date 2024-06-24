-- Drop indexes on chain_id, header_number, and digest
DROP INDEX IF EXISTS idx_chain_id;
DROP INDEX IF EXISTS idx_header_number;
DROP INDEX IF EXISTS idx_digest;

-- Drop the table for SignedAttestation
DROP TABLE IF EXISTS SignedAttestation;
