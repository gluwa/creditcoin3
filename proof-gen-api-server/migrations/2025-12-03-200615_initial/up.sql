-- Create table for continuity_proofs
CREATE TABLE IF NOT EXISTS continuity_proofs (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    continuity_proof JSONB NOT NULL,
    created_at TIMESTAMP DEFAULT now(),
    updated_at TIMESTAMP DEFAULT now()
);

-- Create table for merkle_proofs
CREATE TABLE IF NOT EXISTS merkle_proofs (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    tx_index BIGINT,
    tx_hash VARCHAR(66),
    tx_bytes BYTEA,
    merkle_proof JSONB NOT NULL,
    merkle_root VARCHAR(66) NOT NULL,
    created_at TIMESTAMP DEFAULT now(),
    updated_at TIMESTAMP DEFAULT now()
);

-- Create continuity_proofs indices
CREATE UNIQUE INDEX IF NOT EXISTS continuity_chain_key_header_number ON continuity_proofs (chain_key, header_number);
CREATE INDEX IF NOT EXISTS continuity_created_at ON continuity_proofs (created_at);

-- Create merkle_proofs indices
CREATE UNIQUE INDEX IF NOT EXISTS merkle_unique_null_tx_index ON merkle_proofs (chain_key, header_number) WHERE tx_index IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS merkle_unique_nonnull_tx_index ON merkle_proofs (chain_key, header_number, tx_index) WHERE tx_index IS NOT NULL;
CREATE INDEX IF NOT EXISTS merkle_by_tx_hash ON merkle_proofs (chain_key, tx_hash);
CREATE INDEX IF NOT EXISTS merkle_by_created_at ON merkle_proofs (created_at);