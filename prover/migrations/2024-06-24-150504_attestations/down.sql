-- Drop BlockWithDigest indices
DROP INDEX IF EXISTS block_with_digest_idx_chain_id;
DROP INDEX IF EXISTS block_with_digest_idx_header_number;
DROP INDEX IF EXISTS block_with_digest_idx_digest;

-- Drop Attestation indices
DROP INDEX IF EXISTS attestation_idx_chain_id;
DROP INDEX IF EXISTS attestation_idx_header_number;
DROP INDEX IF EXISTS attestation_idx_digest;

-- Drop AttestationCheckpoint indices
DROP INDEX IF EXISTS attestation_checkpoint_idx_chain_id;
DROP INDEX IF EXISTS attestation_checkpoint_idx_block_number;
DROP INDEX IF EXISTS attestation_checkpoint_idx_digest;

-- Drop the table for BlockWithDigest
DROP TABLE IF EXISTS BlockWithDigest;

-- Drop the table for Attestation
DROP TABLE IF EXISTS Attestation;

-- Drop the table for AttestationCheckpoint
DROP TABLE IF EXISTS AttestationCheckpoint;

-- Drop the table for CachedUpTo
DROP TABLE IF EXISTS CachedUpTo;
