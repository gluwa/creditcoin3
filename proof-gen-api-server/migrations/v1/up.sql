-- Create table for proofs
CREATE TABLE IF NOT EXISTS proofs (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    tx_index BIGINT,
    tx_hash VARCHAR(66),
    continuity_proof JSONB,
    merkle_proof JSONB,
    merkle_root VARCHAR(66),
    created_at TIMESTAMP DEFAULT now(),
    updated_at TIMESTAMP DEFAULT now()
);

-- Create indeces
CREATE UNIQUE INDEX IF NOT EXISTS proofs_unique_null_tx_index ON proofs (chain_key, header_number) WHERE tx_index IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS proofs_unique_nonnull_tx_index ON proofs (chain_key, header_number, tx_index) WHERE tx_index IS NOT NULL;