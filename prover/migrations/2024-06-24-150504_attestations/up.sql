-- Create table for SignedAttestation
CREATE TABLE SignedAttestation (
    id SERIAL PRIMARY KEY,
    chain_id SMALLINT NOT NULL,
    header_number BIGINT NOT NULL,
    header_hash VARCHAR(64) NOT NULL,
    tx_root VARCHAR(64) NOT NULL,
    rx_root VARCHAR(64) NOT NULL,
    digest VARCHAR(64) NOT NULL UNIQUE,
    prev_digest VARCHAR(64) UNIQUE,
    signature VARCHAR(192) NOT NULL,
    attestors TEXT [] NOT NULL
);

-- Create indexes on chain_id, header_number, and digest
CREATE INDEX idx_chain_id ON SignedAttestation (chain_id);
CREATE INDEX idx_header_number ON SignedAttestation (header_number);
CREATE INDEX idx_digest ON SignedAttestation (digest);
