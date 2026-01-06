-- Alter header_number from BIGINT to NUMERIC(20, 0) to prevent i64 overflow
-- for values > i64::MAX
ALTER TABLE continuity_proofs
ALTER COLUMN header_number TYPE NUMERIC(20, 0);
