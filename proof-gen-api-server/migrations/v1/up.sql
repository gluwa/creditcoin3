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

-- Create indexes
CREATE INDEX proofs_idx_chain_and_height ON proofs (chain_key, header_number);
CREATE UNIQUE INDEX proofs_idx_chain_height_and_tx_idx on proofs (chain_key, header_number, tx_index);