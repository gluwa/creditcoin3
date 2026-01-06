-- Revert header_number back to BIGINT
ALTER TABLE continuity_proofs
ALTER COLUMN header_number TYPE BIGINT;
