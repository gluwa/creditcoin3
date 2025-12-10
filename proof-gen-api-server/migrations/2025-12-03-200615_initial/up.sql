-- Create table for continuity_proofs
CREATE TABLE IF NOT EXISTS continuity_proofs (
    id SERIAL PRIMARY KEY,
    chain_key BIGINT NOT NULL,
    header_number BIGINT NOT NULL,
    continuity_proof JSONB NOT NULL,
    ends_in_attestation BOOLEAN NOT NULL,
    created_at TIMESTAMP DEFAULT now(),
    updated_at TIMESTAMP DEFAULT now()
);

-- Create continuity_proofs indices
CREATE UNIQUE INDEX IF NOT EXISTS continuity_chain_key_header_number ON continuity_proofs (chain_key, header_number);
CREATE INDEX IF NOT EXISTS continuity_created_at ON continuity_proofs (created_at);